//! Scan orchestration for Costguard.
//!
//! Loads configuration, discovers project files, parses SQL and dbt metadata,
//! runs the rule engine, applies baselines and signed policy, and returns
//! a [`ScanResult`]. This crate is the integration layer between the CLI and
//! the lower-level parsing, rules, and output crates.

mod analysis_report;
mod base_replay;
mod baseline;
mod branch_scan;
mod config;
mod context;
mod dbt_graph;
mod dbt_load;
mod gates;
mod git;
mod governance;
mod init;
mod metadata_report;
mod owners;
mod pipeline;
mod scan;
mod scan_plan;
mod sql_analysis;
mod state_diff;
mod waivers;

pub use baseline::{
    apply_finding_baseline, load_finding_baseline, merge_baseline_findings,
    validate_finding_baseline, write_finding_baseline, BaselinedFinding, FindingBaseline,
};
pub use config::{
    apply_file_config, load_config, validate_scan_config, AnalysisConfig, AnalysisPolicy,
    AnalysisSection, DbtSection, FileConfig, GateConfig, GateMode, GateScope, OutputFormat,
    OutputSection, OwnerValue, OwnersConfig, ScanConfig, ScanRuntimeOverrides, ScanSection,
    SignedPolicyConfig, SignedPolicySection, Waiver,
};
pub use context::{ContextIssue, ContextReport};
pub use costguard_cost::ProjectCostSummary;
pub use costguard_dbt::{
    DbtColumn, DbtConfig, DbtExposure, DbtGraph, DbtProject as DbtProjectModel, DbtRef, DbtSource,
    DbtSourceRef, DbtSqlFeatures, DbtTest,
};
pub use costguard_diagnostics::{Confidence, LineIndex, Span};
pub use costguard_rules::{Rule, RuleContext, RuleMetadata, RuleOverrides};
pub use costguard_scanner::{FileKind as ProjectFileKind, ProjectFile as ScannedProjectFile};
pub use costguard_sql::Platform;
pub use costguard_sql::{
    CteFeature, ExpressionFeature, JoinFeature, ParseInput, SqlFeatures, WindowFeature,
};
pub use init::{detect_warehouse, init_project, InitOptions, InitOutcome};
pub use scan::{explain, rules, scan};
pub use state_diff::{classify_findings, FindingDelta};

use costguard_dbt::DbtProject;
use costguard_diagnostics::Diagnostic;
use costguard_protocol::EnforcementOutcome;
use costguard_scanner::{ProjectFile, ScanCounts};
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::PathBuf;

/// A discovered project with its classified files and optional dbt metadata.
#[derive(Debug, Clone)]
pub struct Project {
    pub root: PathBuf,
    pub files: Vec<ProjectFile>,
    pub dbt: Option<DbtProject>,
}

/// Parse and scan counters collected during a run.
#[derive(Debug, Clone, Serialize)]
pub struct ScanMetrics {
    pub counts: ScanCounts,
    pub sql_parse_total: usize,
    pub sql_parse_failures: usize,
    pub sql_parse_other_total: usize,
    pub sql_parse_other_failures: usize,
    pub sql_parse_compiled_total: usize,
    pub sql_parse_compiled_failures: usize,
    pub metadata_warnings: usize,
    pub yaml_parse_failures: usize,
    pub dbt_project_parse_failures: usize,
    pub metadata_only_scan: bool,
    pub diagnostics_by_rule: BTreeMap<String, usize>,
    pub diagnostics_by_severity: BTreeMap<String, usize>,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub baselined_findings: usize,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub new_findings: usize,
}

fn is_zero(value: &usize) -> bool {
    *value == 0
}

#[derive(Debug, Clone, Serialize)]
pub struct FileParseStatus {
    pub path: PathBuf,
    pub parse_input: costguard_sql::ParseInput,
    pub parsed: bool,
    pub parsed_raw: bool,
    pub parsed_compiled: bool,
    pub feature_extraction_used_ast: bool,
}

/// Complete output of a scan run, including diagnostics, metrics, and optional PR summary.
#[derive(Debug, Clone)]
pub struct ScanResult {
    pub run: RunMetadata,
    pub policy: PolicyMetadata,
    pub diagnostics: Vec<Diagnostic>,
    pub counts: ScanCounts,
    pub metrics: ScanMetrics,
    pub file_parse_status: Vec<FileParseStatus>,
    pub pr_summary: Option<PrSummary>,
    pub context: Option<ContextReport>,
    pub cost_summary: Option<ProjectCostSummary>,
    pub analysis: AnalysisReport,
    pub identity_scheme: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RunMetadata {
    pub id: String,
    pub started_at: String,
    pub completed_at: String,
    pub duration_ms: u64,
    pub tool_version: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PolicyMetadata {
    pub digest: String,
    pub version: String,
    pub scope: String,
}

impl ScanResult {
    pub fn block_only_new(&self) -> bool {
        self.pr_summary
            .as_ref()
            .and_then(|summary| summary.enforcement_preview.as_ref())
            .is_some_and(|enforcement| enforcement.block_only_new)
    }

    pub fn diagnostic_is_blocking(&self, diagnostic: &Diagnostic) -> bool {
        if !self.block_only_new() {
            return true;
        }
        if costguard_cost::is_infrastructure_rule(&diagnostic.rule_id) {
            return false;
        }
        self.pr_summary
            .as_ref()
            .and_then(|summary| summary.finding_delta.as_ref())
            .is_none_or(|delta| delta.is_blocking(&diagnostic.governance.finding_id))
    }

    pub fn diagnostic_delta_status(&self, diagnostic: &Diagnostic) -> Option<&'static str> {
        if costguard_cost::is_infrastructure_rule(&diagnostic.rule_id) {
            return None;
        }
        let delta = self.pr_summary.as_ref()?.finding_delta.as_ref()?;
        if diagnostic.governance.finding_id.is_empty() {
            Some("unknown")
        } else {
            Some(delta.status(&diagnostic.governance.finding_id))
        }
    }

    /// Returns whether the scan should fail CI given severity, confidence, and cost thresholds.
    pub fn should_fail(
        &self,
        fail_on: Option<costguard_diagnostics::Severity>,
        min_confidence: Option<costguard_diagnostics::Confidence>,
        fail_on_monthly_delta: Option<f64>,
        fail_on_monthly_delta_gb: Option<f64>,
    ) -> bool {
        if !self.analysis.passed {
            return true;
        }
        if self.pr_summary.as_ref().is_some_and(|summary| {
            summary
                .gate_results
                .iter()
                .any(|gate| gate.status == GateStatus::Fail)
        }) {
            return true;
        }
        if let Some(threshold) = fail_on {
            let managed = self.policy.digest != "local-unmanaged";
            if self.diagnostics.iter().any(|diagnostic| {
                diagnostic.severity >= threshold
                    && min_confidence.is_none_or(|mc| diagnostic.confidence >= mc)
                    && diagnostic.governance.enforcement != EnforcementOutcome::Excepted
                    && self.diagnostic_is_blocking(diagnostic)
                    && (!managed
                        || diagnostic.governance.enforcement == EnforcementOutcome::Blocked)
            }) {
                return true;
            }
        }
        if let Some(delta) = fail_on_monthly_delta {
            let savings = if self.block_only_new() {
                self.diagnostics
                    .iter()
                    .filter(|diagnostic| {
                        diagnostic.governance.enforcement != EnforcementOutcome::Excepted
                    })
                    .filter(|diagnostic| self.diagnostic_is_blocking(diagnostic))
                    .filter_map(|diagnostic| {
                        diagnostic
                            .cost_estimate
                            .as_ref()
                            .and_then(|cost| cost.savings_p50_usd_per_month)
                    })
                    .sum()
            } else {
                self.cost_summary
                    .as_ref()
                    .map(|summary| summary.savings_p50_usd)
                    .unwrap_or_else(|| costguard_cost::total_p50_usd_per_month(&self.diagnostics))
            };
            if savings >= delta {
                eprintln!(
                    "cost gate failed: estimated cost impact ${savings:.0}/mo >= threshold ${delta:.0}/mo"
                );
                return true;
            }
        }
        if let Some(delta_gb) = fail_on_monthly_delta_gb {
            let savings_gb = if self.block_only_new() {
                self.diagnostics
                    .iter()
                    .filter(|diagnostic| {
                        diagnostic.governance.enforcement != EnforcementOutcome::Excepted
                    })
                    .filter(|diagnostic| self.diagnostic_is_blocking(diagnostic))
                    .filter_map(|diagnostic| diagnostic.cost_estimate.as_ref())
                    .map(|cost| cost.relative_index)
                    .sum()
            } else {
                self.cost_summary
                    .as_ref()
                    .map(|summary| summary.savings_gb_months)
                    .unwrap_or_else(|| costguard_cost::total_savings_gb_months(&self.diagnostics))
            };
            if savings_gb >= delta_gb {
                eprintln!(
                    "cost gate failed: estimated new savings {savings_gb:.0} GB-mo >= threshold {delta_gb:.0} GB-mo"
                );
                return true;
            }
        }
        false
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct AnalysisReport {
    pub policy: AnalysisPolicy,
    pub passed: bool,
    pub violations: Vec<AnalysisViolation>,
}

impl Default for AnalysisReport {
    fn default() -> Self {
        Self {
            policy: AnalysisPolicy::Standard,
            passed: true,
            violations: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct AnalysisViolation {
    pub code: String,
    pub message: String,
    pub observed: f64,
    pub allowed: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct PrSummary {
    pub receipt_version: u8,
    pub changed_files: Vec<PathBuf>,
    pub changed_models: Vec<String>,
    pub changed_model_details: Vec<ChangedModelDetail>,
    pub owner_summary: BTreeMap<String, usize>,
    pub affected_downstream: Vec<String>,
    pub affected_exposures: Vec<String>,
    pub recommended_dbt_command: Option<String>,
    pub gate_results: Vec<GateResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trend: Option<ReceiptTrend>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finding_delta: Option<FindingDelta>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub indirectly_affected: Vec<IndirectImpact>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manifest_integrity: Option<ManifestIntegrity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enforcement_preview: Option<EnforcementPreview>,
}

#[derive(Debug, Clone, Serialize)]
pub struct IndirectImpact {
    pub model: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ManifestIntegrity {
    pub checksum_mismatches: usize,
    pub verified: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct EnforcementPreview {
    pub block_only_new: bool,
    pub require_manifest_integrity: bool,
}

impl Default for PrSummary {
    fn default() -> Self {
        Self {
            receipt_version: 2,
            changed_files: Vec::new(),
            changed_models: Vec::new(),
            changed_model_details: Vec::new(),
            owner_summary: BTreeMap::new(),
            affected_downstream: Vec::new(),
            affected_exposures: Vec::new(),
            recommended_dbt_command: None,
            gate_results: Vec::new(),
            trend: None,
            finding_delta: None,
            indirectly_affected: Vec::new(),
            manifest_integrity: None,
            enforcement_preview: None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ChangedModelDetail {
    pub id: String,
    pub name: String,
    pub path: PathBuf,
    pub tags: Vec<String>,
    pub owners: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    pub affected_downstream: Vec<String>,
    pub affected_exposures: Vec<String>,
    pub affected_exposure_owners: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum GateStatus {
    Pass,
    Warn,
    Fail,
}

impl std::fmt::Display for GateStatus {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Pass => "pass",
            Self::Warn => "warn",
            Self::Fail => "fail",
        })
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct GateResult {
    pub name: String,
    pub mode: GateMode,
    pub status: GateStatus,
    pub matched_findings: usize,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ReceiptTrend {
    pub diagnostics_delta: i64,
    pub high_findings_delta: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub monthly_savings_delta_usd: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub monthly_savings_delta_gb: Option<f64>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use costguard_diagnostics::{Confidence, Severity};
    use std::path::PathBuf;

    fn diagnostic(severity: Severity, confidence: Confidence) -> Diagnostic {
        Diagnostic {
            governance: Default::default(),
            rule_id: "SQLCOST012".into(),
            severity,
            path: PathBuf::from("models/a.sql"),
            line: 1,
            column: 1,
            span: None,
            message: "test".into(),
            risk: None,
            suggestion: None,
            confidence,
            warehouse: None,
            source_provenance: None,
            compiled_line: None,
            compiled_column: None,
            cost_estimate: None,
            rule_precision_tier: None,
        }
    }

    fn result_with(diagnostics: Vec<Diagnostic>) -> ScanResult {
        ScanResult {
            run: RunMetadata {
                id: "test-run".into(),
                started_at: "2026-01-01T00:00:00Z".into(),
                completed_at: "2026-01-01T00:00:01Z".into(),
                duration_ms: 1,
                tool_version: env!("CARGO_PKG_VERSION").into(),
            },
            policy: PolicyMetadata {
                digest: "local-unmanaged".into(),
                version: env!("CARGO_PKG_VERSION").into(),
                scope: "local".into(),
            },
            diagnostics,
            counts: ScanCounts::default(),
            metrics: ScanMetrics {
                counts: ScanCounts::default(),
                sql_parse_total: 0,
                sql_parse_failures: 0,
                sql_parse_other_total: 0,
                sql_parse_other_failures: 0,
                sql_parse_compiled_total: 0,
                sql_parse_compiled_failures: 0,
                metadata_warnings: 0,
                yaml_parse_failures: 0,
                dbt_project_parse_failures: 0,
                metadata_only_scan: false,
                diagnostics_by_rule: BTreeMap::new(),
                diagnostics_by_severity: BTreeMap::new(),
                baselined_findings: 0,
                new_findings: 0,
            },
            file_parse_status: Vec::new(),
            pr_summary: None,
            context: None,
            cost_summary: None,
            analysis: AnalysisReport::default(),
            identity_scheme: None,
        }
    }

    #[test]
    fn should_fail_respects_min_confidence() {
        let result = result_with(vec![diagnostic(Severity::High, Confidence::Low)]);
        assert!(result.should_fail(Some(Severity::High), None, None, None));
        assert!(!result.should_fail(Some(Severity::High), Some(Confidence::High), None, None));
    }

    #[test]
    fn should_fail_on_cost_delta() {
        let mut diag = diagnostic(Severity::Low, Confidence::High);
        diag.cost_estimate = Some(costguard_diagnostics::CostEstimate {
            relative_index: 100.0,
            grade: costguard_diagnostics::CostGrade::C,
            basis: "test".into(),
            prior_basis: None,
            currency: "USD".into(),
            model_id: None,
            model_monthly_p50_usd: None,
            savings_p10_usd_per_month: Some(400.0),
            savings_p50_usd_per_month: Some(500.0),
            savings_p90_usd_per_month: Some(900.0),
            current_cost_p50_usd_per_month: None,
            post_fix_cost_p50_usd_per_month: None,
            unestimated_reason: None,
            downstream_model_count: None,
            downstream_monthly_p50_usd: None,
        });
        let result = result_with(vec![diag]);
        assert!(!result.should_fail(None, None, Some(600.0), None));
        assert!(result.should_fail(None, None, Some(500.0), None));
    }

    #[test]
    fn should_fail_on_finding_savings_not_pr_impact_net() {
        let mut result = result_with(Vec::new());
        result.cost_summary = Some(costguard_cost::ProjectCostSummary {
            savings_p50_usd: 0.0,
            savings_gb_months: 0.0,
            pr_impact: Some(costguard_cost::PrCostImpact {
                net: costguard_cost::CostFigure {
                    monthly_p50: Some(2_000_000_000_000.0),
                    basis: "pr-net".into(),
                    ..Default::default()
                },
                ..Default::default()
            }),
            ..Default::default()
        });
        assert!(!result.should_fail(None, None, Some(1.0), None));
        assert!(!result.should_fail(None, None, None, Some(1.0)));
    }

    #[test]
    fn top_level_regression_only_gate_ignores_unchanged_finding() {
        let mut unchanged = diagnostic(Severity::High, Confidence::High);
        unchanged.assign_identity("sem-v1:unchanged");
        unchanged.cost_estimate = Some(costguard_diagnostics::CostEstimate {
            relative_index: 100.0,
            grade: costguard_diagnostics::CostGrade::C,
            basis: "test".into(),
            prior_basis: None,
            currency: "USD".into(),
            model_id: None,
            model_monthly_p50_usd: None,
            savings_p10_usd_per_month: Some(400.0),
            savings_p50_usd_per_month: Some(500.0),
            savings_p90_usd_per_month: Some(900.0),
            current_cost_p50_usd_per_month: None,
            post_fix_cost_p50_usd_per_month: None,
            unestimated_reason: None,
            downstream_model_count: None,
            downstream_monthly_p50_usd: None,
        });
        let mut result = result_with(vec![unchanged]);
        result.pr_summary = Some(PrSummary {
            finding_delta: Some(FindingDelta {
                unchanged: 1,
                ..FindingDelta::default()
            }),
            enforcement_preview: Some(EnforcementPreview {
                block_only_new: true,
                require_manifest_integrity: false,
            }),
            ..PrSummary::default()
        });
        assert!(!result.should_fail(
            Some(Severity::High),
            Some(Confidence::High),
            Some(1.0),
            Some(1.0)
        ));

        result
            .pr_summary
            .as_mut()
            .unwrap()
            .enforcement_preview
            .as_mut()
            .unwrap()
            .block_only_new = false;
        assert!(result.should_fail(Some(Severity::High), None, None, None));
    }

    #[test]
    fn top_level_regression_only_gate_fails_closed_without_identity() {
        let mut result = result_with(vec![diagnostic(Severity::High, Confidence::High)]);
        result.pr_summary = Some(PrSummary {
            finding_delta: Some(FindingDelta::default()),
            enforcement_preview: Some(EnforcementPreview {
                block_only_new: true,
                require_manifest_integrity: false,
            }),
            ..PrSummary::default()
        });
        assert!(result.should_fail(Some(Severity::High), None, None, None));
    }
}
