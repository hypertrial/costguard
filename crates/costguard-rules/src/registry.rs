use costguard_dbt::DbtModel;
use costguard_diagnostics::{Diagnostic, Severity};
use costguard_scanner::ProjectFile;
use costguard_sql::SqlDocument;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub use costguard_platform::Platform;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuleOverride {
    pub enabled: Option<bool>,
    pub severity: Option<Severity>,
    pub threshold: Option<usize>,
}

pub type RuleOverrides = HashMap<String, RuleOverride>;

#[derive(Debug, Clone, Default)]
pub struct ProjectIndexes {
    pub expensive_expression_file_counts: HashMap<String, usize>,
}

impl ProjectIndexes {
    pub fn from_sql_documents(sql_documents: &[SqlDocument]) -> Self {
        let mut file_counts = HashMap::new();
        for doc in sql_documents {
            let mut seen = std::collections::HashSet::new();
            for feature in doc
                .features
                .json_extractions
                .iter()
                .chain(doc.features.regex_calls.iter())
                .chain(doc.features.normalization_calls.iter())
            {
                seen.insert(feature.key.clone());
            }
            for key in seen {
                *file_counts.entry(key).or_default() += 1;
            }
        }
        Self {
            expensive_expression_file_counts: file_counts,
        }
    }
}

pub struct RuleContext<'a> {
    pub warehouse: Platform,
    pub file: &'a ProjectFile,
    pub sql: Option<&'a SqlDocument>,
    pub dbt_model: Option<&'a DbtModel>,
    pub all_sql: &'a [SqlDocument],
    pub project_indexes: &'a ProjectIndexes,
    pub overrides: &'a RuleOverrides,
}

pub trait Rule: Send + Sync {
    fn id(&self) -> &'static str;
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    fn default_severity(&self) -> Severity;

    fn applies_to(&self, _platform: Platform) -> bool {
        true
    }

    fn check(&self, ctx: &RuleContext<'_>) -> Vec<Diagnostic>;
}

#[derive(Debug, Clone, Serialize)]
pub struct RuleMetadata {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub severity: Severity,
}

pub struct RuleRegistry {
    rules: Vec<Box<dyn Rule>>,
}

impl RuleRegistry {
    pub fn default_rules() -> Self {
        Self {
            rules: vec![
                Box::new(crate::sql_shape::SelectStarRule),
                Box::new(crate::expressions::RepeatedJsonRule),
                Box::new(crate::expressions::RepeatedRegexRule),
                Box::new(crate::dbt::IncrementalUniqueKeyRule),
                Box::new(crate::dbt::IncrementalPredicateRule),
                Box::new(crate::sql_shape::UnboundedJoinRule),
                Box::new(crate::sql_shape::OrderByIntermediateRule),
                Box::new(crate::sql_shape::BlindDistinctRule),
                Box::new(crate::expressions::RepeatedNormalizationRule),
                Box::new(crate::python::PythonRowWiseRule),
                Box::new(crate::dbt::SourceInMartRule),
                Box::new(crate::sql_shape::CrossJoinRule),
                Box::new(crate::sql_shape::UnpartitionedWindowRule),
                Box::new(crate::sql_shape::RepeatedCteRule),
                Box::new(crate::expressions::RepeatedExpensiveAcrossFilesRule),
                Box::new(crate::cost_patterns::NonSargablePredicateRule),
                Box::new(crate::cost_patterns::FunctionWrappedJoinKeyRule),
                Box::new(crate::cost_patterns::UnionWithoutAllRule),
                Box::new(crate::dbt::IncrementalSourceBoundRule),
                Box::new(crate::cost_patterns::CountDistinctRule),
                Box::new(crate::cost_patterns::WildcardTableScanRule),
                Box::new(crate::python::PythonLocalCollectionRule),
                Box::new(crate::metadata::MetadataOnlyScanRule),
                Box::new(crate::metadata::YamlParseFailedRule),
                Box::new(crate::metadata::DbtProjectMetadataRule),
                Box::new(crate::metadata::FileSkippedRule),
                Box::new(crate::metadata::SqlParseFailedRule),
                Box::new(crate::dbt::MissingPartitionClusterRule),
                Box::new(crate::dbt::FullRefreshHeavyIncrementalRule),
                Box::new(crate::cost_patterns::CorrelatedSubqueryRule),
                Box::new(crate::cost_patterns::LeadingWildcardLikeRule),
                Box::new(crate::cost_patterns::OrPartitionPredicateRule),
                Box::new(crate::cost_patterns::PatternMatchingJoinRule),
                Box::new(crate::cost_patterns::ScalarSubqueryInSelectRule),
                Box::new(crate::cost_patterns::CrossCatalogJoinRule),
            ],
        }
    }

    pub fn metadata(&self) -> Vec<RuleMetadata> {
        self.rules
            .iter()
            .map(|rule| RuleMetadata {
                id: rule.id(),
                name: rule.name(),
                description: rule.description(),
                severity: rule.default_severity(),
            })
            .collect()
    }

    pub fn run(&self, ctx: &RuleContext<'_>) -> Vec<Diagnostic> {
        self.rules
            .iter()
            .filter(|rule| rule.applies_to(ctx.warehouse))
            .filter(|rule| {
                ctx.overrides
                    .get(rule.id())
                    .and_then(|settings| settings.enabled)
                    .unwrap_or(true)
            })
            .flat_map(|rule| {
                let mut diagnostics = rule.check(ctx);
                if let Some(settings) = ctx.overrides.get(rule.id()) {
                    if let Some(severity) = settings.severity {
                        for diagnostic in &mut diagnostics {
                            diagnostic.severity = severity;
                        }
                    }
                }
                diagnostics
            })
            .collect()
    }
}
