mod config;
mod dbt_graph;
mod scan;

pub use config::{
    apply_file_config, load_config, DbtSection, FileConfig, OutputFormat, OutputSection,
    ScanConfig, ScanSection,
};
pub use costguard_dbt::{
    DbtColumn, DbtConfig, DbtExposure, DbtGraph, DbtProject as DbtProjectModel, DbtRef, DbtSource,
    DbtSourceRef, DbtSqlFeatures, DbtTest,
};
pub use costguard_diagnostics::{Confidence, LineIndex, Span};
pub use costguard_platform::Platform;
pub use costguard_rules::{Rule, RuleContext, RuleMetadata, RuleOverrides, Warehouse};
pub use costguard_scanner::{FileKind as ProjectFileKind, ProjectFile as ScannedProjectFile};
pub use costguard_sql::{
    CteFeature, ExpressionFeature, JoinFeature, SqlDialect, SqlFeatures, WindowFeature,
};
pub use scan::{explain, rules, scan};

use costguard_dbt::DbtProject;
use costguard_diagnostics::Diagnostic;
use costguard_scanner::{ProjectFile, ScanCounts};
use serde::Serialize;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Project {
    pub root: PathBuf,
    pub files: Vec<ProjectFile>,
    pub dbt: Option<DbtProject>,
}

#[derive(Debug, Clone)]
pub struct ScanResult {
    pub diagnostics: Vec<Diagnostic>,
    pub counts: ScanCounts,
    pub pr_summary: Option<PrSummary>,
}

impl ScanResult {
    pub fn should_fail(&self, fail_on: Option<costguard_diagnostics::Severity>) -> bool {
        let Some(threshold) = fail_on else {
            return false;
        };
        self.diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity >= threshold)
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
