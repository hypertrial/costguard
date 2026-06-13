mod baseline;
mod config;
mod dbt_graph;
mod dbt_load;
mod scan;
mod sql_analysis;

pub use baseline::{
    apply_finding_baseline, load_finding_baseline, load_legacy_baseline_v1,
    merge_baseline_findings, migrate_legacy_baseline_v1, validate_finding_baseline,
    write_finding_baseline, BaselinedFinding, FindingBaseline, LegacyFindingBaselineV1,
};
pub use config::{
    apply_file_config, load_config, validate_scan_config, AnalysisConfig, AnalysisPolicy,
    AnalysisSection, DbtSection, EnterprisePolicyConfig, EnterprisePolicySection, FileConfig,
    OutputFormat, OutputSection, ScanConfig, ScanRuntimeOverrides, ScanSection,
};
pub use costguard_cost::CostConfig;
pub use costguard_cost::ProjectCostSummary;
pub use costguard_dbt::{
    DbtColumn, DbtConfig, DbtExposure, DbtGraph, DbtProject as DbtProjectModel, DbtRef, DbtSource,
    DbtSourceRef, DbtSqlFeatures, DbtTest,
};
pub use costguard_diagnostics::{Confidence, LineIndex, Span};
pub use costguard_platform::Platform;
pub use costguard_protocol::{RepositoryRefV1, ScanEnvelopeV1};
pub use costguard_rules::{Rule, RuleContext, RuleMetadata, RuleOverrides};
pub use costguard_scanner::{FileKind as ProjectFileKind, ProjectFile as ScannedProjectFile};
pub use costguard_sql::{
    CteFeature, ExpressionFeature, JoinFeature, ParseInput, SqlFeatures, WindowFeature,
};
pub use scan::{explain, rules, scan};

use costguard_dbt::DbtProject;
use costguard_diagnostics::Diagnostic;
use costguard_protocol::EnforcementOutcome;
use costguard_scanner::{ProjectFile, ScanCounts};
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Project {
    pub root: PathBuf,
    pub files: Vec<ProjectFile>,
    pub dbt: Option<DbtProject>,
}

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

#[derive(Debug, Clone)]
pub struct ScanResult {
    pub run: RunMetadata,
    pub policy: PolicyMetadata,
    pub diagnostics: Vec<Diagnostic>,
    pub counts: ScanCounts,
    pub metrics: ScanMetrics,
    pub file_parse_status: Vec<FileParseStatus>,
    pub pr_summary: Option<PrSummary>,
    pub cost_summary: Option<ProjectCostSummary>,
    pub analysis: AnalysisReport,
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
    pub fn to_envelope(
        &self,
        repository: costguard_protocol::RepositoryRefV1,
        attempt: u32,
    ) -> anyhow::Result<costguard_protocol::ScanEnvelopeV1> {
        let findings = self
            .diagnostics
            .iter()
            .map(|diagnostic| {
                Ok(costguard_protocol::FindingV1 {
                    finding_id: diagnostic.governance.finding_id.clone(),
                    evidence_key: diagnostic.governance.evidence_key.clone(),
                    rule_id: diagnostic.rule_id.clone(),
                    severity: diagnostic.severity.to_string(),
                    confidence: match diagnostic.confidence {
                        costguard_diagnostics::Confidence::Low => "low",
                        costguard_diagnostics::Confidence::Medium => "medium",
                        costguard_diagnostics::Confidence::High => "high",
                    }
                    .into(),
                    path: diagnostic.path.to_string_lossy().replace('\\', "/"),
                    line: diagnostic.line,
                    column: diagnostic.column,
                    message: diagnostic.message.clone(),
                    enforcement: diagnostic.governance.enforcement,
                    policy: diagnostic.governance.policy.clone(),
                    exception: diagnostic.governance.exception.clone(),
                    cost: diagnostic
                        .cost_estimate
                        .as_ref()
                        .map(serde_json::to_value)
                        .transpose()?,
                })
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        let files = self
            .file_parse_status
            .iter()
            .map(|file| costguard_protocol::FileStatusV1 {
                path: file.path.to_string_lossy().replace('\\', "/"),
                parse_input: match file.parse_input {
                    costguard_sql::ParseInput::Raw => "raw",
                    costguard_sql::ParseInput::Compiled => "compiled",
                    costguard_sql::ParseInput::CompiledWithRawFallback => {
                        "compiled_with_raw_fallback"
                    }
                }
                .into(),
                parsed: file.parsed,
                feature_extraction_used_ast: file.feature_extraction_used_ast,
            })
            .collect();
        Ok(costguard_protocol::ScanEnvelopeV1 {
            schema_version: costguard_protocol::SCAN_SCHEMA_VERSION,
            run: costguard_protocol::ScanRunV1 {
                id: self.run.id.clone(),
                attempt,
                started_at: self.run.started_at.clone(),
                completed_at: self.run.completed_at.clone(),
                duration_ms: self.run.duration_ms,
                tool_version: self.run.tool_version.clone(),
            },
            repository,
            policy_digest: self.policy.digest.clone(),
            analysis: serde_json::to_value(&self.analysis)?,
            metrics: serde_json::to_value(&self.metrics)?,
            cost: self
                .cost_summary
                .as_ref()
                .map(serde_json::to_value)
                .transpose()?,
            findings,
            files,
            pr_summary: self
                .pr_summary
                .as_ref()
                .map(serde_json::to_value)
                .transpose()?,
        })
    }

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
        if let Some(threshold) = fail_on {
            let managed = self.policy.digest != "local-unmanaged";
            if self.diagnostics.iter().any(|diagnostic| {
                diagnostic.severity >= threshold
                    && min_confidence.is_none_or(|mc| diagnostic.confidence >= mc)
                    && (!managed
                        || diagnostic.governance.enforcement == EnforcementOutcome::Blocked)
            }) {
                return true;
            }
        }
        if let Some(delta) = fail_on_monthly_delta {
            let savings = costguard_cost::total_p50_usd_per_month(&self.diagnostics);
            if savings >= delta {
                eprintln!(
                    "cost gate failed: estimated new savings ${savings:.0}/mo >= threshold ${delta:.0}/mo"
                );
                return true;
            }
        }
        if let Some(delta_gb) = fail_on_monthly_delta_gb {
            let savings_gb = costguard_cost::total_savings_gb_months(&self.diagnostics);
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

#[derive(Debug, Clone, Default, Serialize)]
pub struct PrSummary {
    pub changed_files: Vec<PathBuf>,
    pub changed_models: Vec<String>,
    pub affected_downstream: Vec<String>,
    pub affected_exposures: Vec<String>,
    pub recommended_dbt_command: Option<String>,
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
            cost_summary: None,
            analysis: AnalysisReport::default(),
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
            currency: "USD".into(),
            model_id: None,
            model_monthly_p50_usd: None,
            savings_p10_usd_per_month: Some(400.0),
            savings_p50_usd_per_month: Some(500.0),
            savings_p90_usd_per_month: Some(900.0),
        });
        let result = result_with(vec![diag]);
        assert!(!result.should_fail(None, None, Some(600.0), None));
        assert!(result.should_fail(None, None, Some(500.0), None));
    }

    #[test]
    fn envelope_contains_metadata_without_source_content_fields() {
        let result = result_with(vec![diagnostic(Severity::High, Confidence::High)]);
        let envelope = result
            .to_envelope(
                costguard_protocol::RepositoryRefV1 {
                    organization: "acme".into(),
                    repository: "warehouse".into(),
                    commit_sha: "abc123".into(),
                    pull_request: Some(42),
                    base_sha: Some("def456".into()),
                },
                1,
            )
            .unwrap();
        let value = serde_json::to_value(envelope).unwrap();
        let object = value.as_object().unwrap();
        for forbidden in [
            "sql",
            "yaml",
            "python",
            "manifest",
            "snippet",
            "source_text",
        ] {
            assert!(!object.contains_key(forbidden));
        }
    }
}
