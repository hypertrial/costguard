use crate::registry::{Rule, RuleContext};
use costguard_diagnostics::{Diagnostic, Severity};

pub(crate) struct MetadataOnlyScanRule;
pub(crate) struct YamlParseFailedRule;
pub(crate) struct DbtProjectMetadataRule;
pub(crate) struct FileSkippedRule;
pub(crate) struct SqlParseFailedRule;

impl Rule for MetadataOnlyScanRule {
    fn id(&self) -> &'static str {
        "SQLCOST023"
    }
    fn name(&self) -> &'static str {
        "Scan without dbt manifest"
    }
    fn description(&self) -> &'static str {
        "Reports when Costguard scans dbt metadata from YAML/SQL only without a manifest."
    }
    fn default_severity(&self) -> Severity {
        Severity::Info
    }
    fn check(&self, _ctx: &RuleContext<'_>) -> Vec<Diagnostic> {
        Vec::new()
    }
}

impl Rule for YamlParseFailedRule {
    fn id(&self) -> &'static str {
        "SQLCOST024"
    }
    fn name(&self) -> &'static str {
        "Schema YAML parse failure"
    }
    fn description(&self) -> &'static str {
        "Reports when a dbt schema YAML file failed to parse."
    }
    fn default_severity(&self) -> Severity {
        Severity::Low
    }
    fn check(&self, _ctx: &RuleContext<'_>) -> Vec<Diagnostic> {
        Vec::new()
    }
}

impl Rule for DbtProjectMetadataRule {
    fn id(&self) -> &'static str {
        "SQLCOST025"
    }
    fn name(&self) -> &'static str {
        "dbt_project.yml metadata issue"
    }
    fn description(&self) -> &'static str {
        "Reports when dbt_project.yml failed to parse or has an ambiguous models block."
    }
    fn default_severity(&self) -> Severity {
        Severity::Low
    }
    fn check(&self, _ctx: &RuleContext<'_>) -> Vec<Diagnostic> {
        Vec::new()
    }
}

impl Rule for FileSkippedRule {
    fn id(&self) -> &'static str {
        "SQLCOST026"
    }
    fn name(&self) -> &'static str {
        "File skipped during scan"
    }
    fn description(&self) -> &'static str {
        "Reports when a SQL or dbt file exceeds the configured scan size limit and was not loaded."
    }
    fn default_severity(&self) -> Severity {
        Severity::Low
    }
    fn check(&self, _ctx: &RuleContext<'_>) -> Vec<Diagnostic> {
        Vec::new()
    }
}

impl Rule for SqlParseFailedRule {
    fn id(&self) -> &'static str {
        "SQLCOST027"
    }
    fn name(&self) -> &'static str {
        "SQL parse failure"
    }
    fn description(&self) -> &'static str {
        "Reports when a dbt model SQL file could not be parsed and rules may fall back to regex heuristics."
    }
    fn default_severity(&self) -> Severity {
        Severity::Info
    }
    fn check(&self, _ctx: &RuleContext<'_>) -> Vec<Diagnostic> {
        Vec::new()
    }
}
