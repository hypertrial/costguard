use costguard_dbt::DbtModel;
use costguard_diagnostics::{Diagnostic, Severity};
use costguard_scanner::ProjectFile;
use costguard_sql::SqlDocument;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Warehouse {
    #[default]
    Generic,
    Snowflake,
    BigQuery,
    Databricks,
    Redshift,
    Postgres,
    DuckDB,
}

impl FromStr for Warehouse {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "generic" => Ok(Self::Generic),
            "snowflake" => Ok(Self::Snowflake),
            "bigquery" => Ok(Self::BigQuery),
            "databricks" => Ok(Self::Databricks),
            "redshift" => Ok(Self::Redshift),
            "postgres" | "postgresql" => Ok(Self::Postgres),
            "duckdb" => Ok(Self::DuckDB),
            other => Err(format!("unknown warehouse '{other}'")),
        }
    }
}

impl std::fmt::Display for Warehouse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            Self::Generic => "generic",
            Self::Snowflake => "snowflake",
            Self::BigQuery => "bigquery",
            Self::Databricks => "databricks",
            Self::Redshift => "redshift",
            Self::Postgres => "postgres",
            Self::DuckDB => "duckdb",
        };
        f.write_str(value)
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
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
    pub warehouse: Warehouse,
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

    fn applies_to(&self, _warehouse: Warehouse) -> bool {
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
