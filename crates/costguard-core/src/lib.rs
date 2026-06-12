mod baseline;
mod config;
mod dbt_graph;
mod dbt_load;
mod scan;
mod sql_analysis;

pub use baseline::{
    apply_finding_baseline, diagnostic_fingerprint, load_finding_baseline, write_finding_baseline,
    BaselinedFinding, FindingBaseline,
};
pub use config::{
    apply_file_config, load_config, validate_scan_config, DbtSection, FileConfig, OutputFormat,
    OutputSection, ScanConfig, ScanRuntimeOverrides, ScanSection,
};
pub use costguard_cost::CostConfig;
pub use costguard_cost::ProjectCostSummary;
pub use costguard_dbt::{
    DbtColumn, DbtConfig, DbtExposure, DbtGraph, DbtProject as DbtProjectModel, DbtRef, DbtSource,
    DbtSourceRef, DbtSqlFeatures, DbtTest,
};
pub use costguard_diagnostics::{Confidence, LineIndex, Span};
pub use costguard_platform::Platform;
pub use costguard_rules::{Rule, RuleContext, RuleMetadata, RuleOverrides, Warehouse};
pub use costguard_scanner::{FileKind as ProjectFileKind, ProjectFile as ScannedProjectFile};
pub use costguard_sql::{
    CteFeature, ExpressionFeature, JoinFeature, ParseInput, SqlDialect, SqlFeatures, WindowFeature,
};
pub use scan::{explain, rules, scan};

use costguard_dbt::DbtProject;
use costguard_diagnostics::Diagnostic;
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
    pub diagnostics: Vec<Diagnostic>,
    pub counts: ScanCounts,
    pub metrics: ScanMetrics,
    pub file_parse_status: Vec<FileParseStatus>,
    pub pr_summary: Option<PrSummary>,
    pub cost_summary: Option<ProjectCostSummary>,
}

impl ScanResult {
    pub fn should_fail(
        &self,
        fail_on: Option<costguard_diagnostics::Severity>,
        min_confidence: Option<costguard_diagnostics::Confidence>,
        fail_on_monthly_delta: Option<f64>,
        fail_on_monthly_delta_gb: Option<f64>,
    ) -> bool {
        if let Some(threshold) = fail_on {
            if self.diagnostics.iter().any(|diagnostic| {
                diagnostic.severity >= threshold
                    && min_confidence.is_none_or(|mc| diagnostic.confidence >= mc)
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
            p10_usd_per_month: Some(400.0),
            p50_usd_per_month: Some(500.0),
            p90_usd_per_month: Some(900.0),
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
}
