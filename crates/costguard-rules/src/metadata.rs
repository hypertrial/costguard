use crate::registry::{Rule, RuleContext};
use costguard_diagnostics::{Diagnostic, Severity};

pub(crate) struct MetadataOnlyScanRule;
pub(crate) struct YamlParseFailedRule;
pub(crate) struct DbtProjectMetadataRule;

impl Rule for MetadataOnlyScanRule {
    fn id(&self) -> &'static str {
        "SQLCOST016"
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
        "SQLCOST017"
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
        "SQLCOST018"
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
