use costguard_dbt::DbtModel;
use costguard_diagnostics::{Confidence, Diagnostic, Severity};
use costguard_scanner::ProjectFile;
use costguard_sql::{JoinKind, SqlDocument};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
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

pub struct RuleContext<'a> {
    pub warehouse: Warehouse,
    pub file: &'a ProjectFile,
    pub sql: Option<&'a SqlDocument>,
    pub dbt_model: Option<&'a DbtModel>,
    pub all_sql: &'a [SqlDocument],
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
                Box::new(SelectStarRule),
                Box::new(RepeatedJsonRule),
                Box::new(RepeatedRegexRule),
                Box::new(IncrementalUniqueKeyRule),
                Box::new(IncrementalPredicateRule),
                Box::new(UnboundedJoinRule),
                Box::new(OrderByIntermediateRule),
                Box::new(BlindDistinctRule),
                Box::new(RepeatedNormalizationRule),
                Box::new(PythonRowWiseRule),
                Box::new(SourceInMartRule),
                Box::new(CrossJoinRule),
                Box::new(UnpartitionedWindowRule),
                Box::new(RepeatedCteRule),
                Box::new(RepeatedExpensiveAcrossFilesRule),
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

struct SelectStarRule;
struct RepeatedJsonRule;
struct RepeatedRegexRule;
struct IncrementalUniqueKeyRule;
struct IncrementalPredicateRule;
struct UnboundedJoinRule;
struct OrderByIntermediateRule;
struct BlindDistinctRule;
struct RepeatedNormalizationRule;
struct PythonRowWiseRule;
struct SourceInMartRule;
struct CrossJoinRule;
struct UnpartitionedWindowRule;
struct RepeatedCteRule;
struct RepeatedExpensiveAcrossFilesRule;

impl Rule for SelectStarRule {
    fn id(&self) -> &'static str {
        "SQLCOST001"
    }
    fn name(&self) -> &'static str {
        "SELECT * in non-staging model"
    }
    fn description(&self) -> &'static str {
        "Detects SELECT * in downstream dbt models."
    }
    fn default_severity(&self) -> Severity {
        Severity::Medium
    }
    fn check(&self, ctx: &RuleContext<'_>) -> Vec<Diagnostic> {
        let Some(sql) = ctx.sql else {
            return Vec::new();
        };
        if !is_downstream_model(&ctx.file.root_relative_path)
            || is_staging_model(&ctx.file.root_relative_path)
        {
            return Vec::new();
        }
        sql.features
            .select_stars
            .iter()
            .map(|feature| {
                diagnostic(
                    ctx,
                    self.id(),
                    self.default_severity(),
                    Some(feature.span),
                    "SELECT * in a non-staging model.",
                )
                .with_risk(
                    "schema drift and unnecessary column scans can increase downstream cost.",
                )
                .with_suggestion(
                    "select required columns explicitly; for dbt, consider model contracts.",
                )
            })
            .collect()
    }
}

impl Rule for RepeatedJsonRule {
    fn id(&self) -> &'static str {
        "SQLCOST002"
    }
    fn name(&self) -> &'static str {
        "Repeated JSON extraction"
    }
    fn description(&self) -> &'static str {
        "Detects repeated semi-structured extraction in one file."
    }
    fn default_severity(&self) -> Severity {
        Severity::Medium
    }
    fn check(&self, ctx: &RuleContext<'_>) -> Vec<Diagnostic> {
        let Some(sql) = ctx.sql else {
            return Vec::new();
        };
        let threshold = threshold(ctx, self.id(), 2);
        let mut counts: HashMap<&str, usize> = HashMap::new();
        for feature in &sql.features.json_extractions {
            *counts.entry(&feature.key).or_default() += 1;
        }
        sql.features
            .json_extractions
            .iter()
            .filter(|feature| counts.get(feature.key.as_str()).copied().unwrap_or(0) >= threshold)
            .take(1)
            .map(|feature| {
                let severity = if is_staging_model(&ctx.file.root_relative_path)
                    && ctx.file.text.to_ascii_lowercase().contains("raw")
                {
                    Severity::High
                } else {
                    self.default_severity()
                };
                diagnostic(
                    ctx,
                    self.id(),
                    severity,
                    Some(feature.span),
                    "Repeated JSON extraction detected.",
                )
                .with_risk("repeated semi-structured parsing can increase warehouse compute.")
                .with_suggestion("materialize extracted fields once in staging.")
            })
            .collect()
    }
}

impl Rule for RepeatedRegexRule {
    fn id(&self) -> &'static str {
        "SQLCOST003"
    }
    fn name(&self) -> &'static str {
        "Repeated regex extraction or replacement"
    }
    fn description(&self) -> &'static str {
        "Detects repeated or excessive regex work."
    }
    fn default_severity(&self) -> Severity {
        Severity::Medium
    }
    fn check(&self, ctx: &RuleContext<'_>) -> Vec<Diagnostic> {
        let Some(sql) = ctx.sql else {
            return Vec::new();
        };
        let threshold = threshold(ctx, self.id(), 2);
        if sql.features.regex_calls.len() < threshold {
            return Vec::new();
        }
        sql.features
            .regex_calls
            .first()
            .map_or_else(Vec::new, |feature| {
                vec![diagnostic(
                    ctx,
                    self.id(),
                    self.default_severity(),
                    Some(feature.span),
                    "Repeated regex operations detected.",
                )
                .with_risk(
                    "regex extraction and replacement are often expensive at warehouse scale.",
                )
                .with_suggestion(
                    "replace simple regex with split/substr/position, or pre-tokenize upstream.",
                )]
            })
    }
}

impl Rule for IncrementalUniqueKeyRule {
    fn id(&self) -> &'static str {
        "SQLCOST004"
    }
    fn name(&self) -> &'static str {
        "Incremental model without unique_key"
    }
    fn description(&self) -> &'static str {
        "Detects dbt incremental models without a unique key."
    }
    fn default_severity(&self) -> Severity {
        Severity::High
    }
    fn check(&self, ctx: &RuleContext<'_>) -> Vec<Diagnostic> {
        let Some(sql) = ctx.sql else {
            return Vec::new();
        };
        let materialized = sql.dbt.config.materialized.as_deref().or(ctx
            .dbt_model
            .and_then(|model| model.materialized.as_deref()));
        let unique_key = sql
            .dbt
            .config
            .unique_key
            .as_deref()
            .or(ctx.dbt_model.and_then(|model| model.unique_key.as_deref()));
        if materialized == Some("incremental") && unique_key.is_none() {
            vec![diagnostic(
                ctx,
                self.id(),
                self.default_severity(),
                None,
                "Incremental model appears to have no unique_key.",
            )
            .with_risk("merge/update logic may be unsafe or require full scans.")
            .with_suggestion("define config(unique_key=...) or document why this is append-only.")]
        } else {
            Vec::new()
        }
    }
}

impl Rule for IncrementalPredicateRule {
    fn id(&self) -> &'static str {
        "SQLCOST005"
    }
    fn name(&self) -> &'static str {
        "Incremental model without date or partition predicate"
    }
    fn description(&self) -> &'static str {
        "Detects incremental models without an obvious pruning predicate."
    }
    fn default_severity(&self) -> Severity {
        Severity::High
    }
    fn check(&self, ctx: &RuleContext<'_>) -> Vec<Diagnostic> {
        let Some(sql) = ctx.sql else {
            return Vec::new();
        };
        let materialized = sql.dbt.config.materialized.as_deref().or(ctx
            .dbt_model
            .and_then(|model| model.materialized.as_deref()));
        if materialized != Some("incremental") || !sql.dbt.uses_is_incremental {
            return Vec::new();
        }
        let lower = ctx.file.text.to_ascii_lowercase();
        let has_predicate = [
            "updated_at",
            "created_at",
            "event_date",
            "ingested_at",
            "_partitiontime",
            "_partitiondate",
            "partition_date",
        ]
        .iter()
        .any(|needle| lower.contains(needle));
        if has_predicate {
            return Vec::new();
        }
        vec![diagnostic(
            ctx,
            self.id(),
            self.default_severity(),
            None,
            "Incremental model has no obvious date or partition predicate.",
        )
        .with_risk("incremental runs may scan much more source data than intended.")
        .with_suggestion(incremental_predicate_suggestion(ctx.warehouse))]
    }
}

impl Rule for UnboundedJoinRule {
    fn id(&self) -> &'static str {
        "SQLCOST006"
    }
    fn name(&self) -> &'static str {
        "Unbounded join risk"
    }
    fn description(&self) -> &'static str {
        "Detects joins without safe equality predicates."
    }
    fn default_severity(&self) -> Severity {
        Severity::Medium
    }
    fn check(&self, ctx: &RuleContext<'_>) -> Vec<Diagnostic> {
        let Some(sql) = ctx.sql else {
            return Vec::new();
        };
        sql.features
            .joins
            .iter()
            .filter(|join| {
                matches!(
                    join.kind,
                    JoinKind::Inner | JoinKind::Left | JoinKind::Right | JoinKind::Full
                ) && (!join.has_equality || join.function_on_both_sides)
            })
            .map(|join| {
                diagnostic(
                    ctx,
                    self.id(),
                    self.default_severity(),
                    Some(join.span),
                    "Join has no clear equality predicate.",
                )
                .with_risk(
                    "non-equality or function-wrapped joins can cause large intermediate scans.",
                )
                .with_suggestion("join on stable keys before applying transformations or filters.")
                .with_confidence(Confidence::Medium)
            })
            .collect()
    }
}

impl Rule for OrderByIntermediateRule {
    fn id(&self) -> &'static str {
        "SQLCOST007"
    }
    fn name(&self) -> &'static str {
        "ORDER BY in model"
    }
    fn description(&self) -> &'static str {
        "Detects ORDER BY in non-final models without LIMIT."
    }
    fn default_severity(&self) -> Severity {
        Severity::Low
    }
    fn check(&self, ctx: &RuleContext<'_>) -> Vec<Diagnostic> {
        let Some(sql) = ctx.sql else {
            return Vec::new();
        };
        if !is_downstream_model(&ctx.file.root_relative_path)
            || ctx.file.text.to_ascii_lowercase().contains("limit ")
        {
            return Vec::new();
        }
        sql.features
            .order_by_clauses
            .iter()
            .map(|feature| {
                diagnostic(
                    ctx,
                    self.id(),
                    self.default_severity(),
                    Some(feature.span),
                    "ORDER BY in a model without LIMIT.",
                )
                .with_risk("sorting intermediate results can add avoidable warehouse work.")
                .with_suggestion("remove unless it is required for deterministic downstream logic.")
            })
            .collect()
    }
}

impl Rule for BlindDistinctRule {
    fn id(&self) -> &'static str {
        "SQLCOST008"
    }
    fn name(&self) -> &'static str {
        "Blind SELECT DISTINCT"
    }
    fn description(&self) -> &'static str {
        "Detects SELECT DISTINCT deduplication."
    }
    fn default_severity(&self) -> Severity {
        Severity::Medium
    }
    fn check(&self, ctx: &RuleContext<'_>) -> Vec<Diagnostic> {
        let Some(sql) = ctx.sql else {
            return Vec::new();
        };
        sql.features
            .distincts
            .iter()
            .map(|feature| {
                diagnostic(ctx, self.id(), self.default_severity(), Some(feature.span), "SELECT DISTINCT detected.")
                    .with_risk("blind deduplication can hide upstream data quality issues and force large sorts.")
                    .with_suggestion("deduplicate explicitly with keys and row_number() when that is the intent.")
            })
            .collect()
    }
}

impl Rule for RepeatedNormalizationRule {
    fn id(&self) -> &'static str {
        "SQLCOST009"
    }
    fn name(&self) -> &'static str {
        "Repeated normalization expression"
    }
    fn description(&self) -> &'static str {
        "Detects repeated lower/upper trim normalization."
    }
    fn default_severity(&self) -> Severity {
        Severity::Low
    }
    fn check(&self, ctx: &RuleContext<'_>) -> Vec<Diagnostic> {
        let Some(sql) = ctx.sql else {
            return Vec::new();
        };
        let mut counts: HashMap<&str, usize> = HashMap::new();
        for feature in &sql.features.normalization_calls {
            *counts.entry(&feature.key).or_default() += 1;
        }
        sql.features
            .normalization_calls
            .iter()
            .find(|feature| counts.get(feature.key.as_str()).copied().unwrap_or(0) > 1)
            .map_or_else(Vec::new, |feature| {
                vec![diagnostic(
                    ctx,
                    self.id(),
                    self.default_severity(),
                    Some(feature.span),
                    "Repeated normalization expression detected.",
                )
                .with_risk(
                    "repeated canonicalization adds compute and makes logic harder to audit.",
                )
                .with_suggestion("canonicalize once upstream.")]
            })
    }
}

impl Rule for PythonRowWiseRule {
    fn id(&self) -> &'static str {
        "SQLCOST010"
    }
    fn name(&self) -> &'static str {
        "Python model row-wise operation"
    }
    fn description(&self) -> &'static str {
        "Detects row-wise pandas patterns in Python dbt models."
    }
    fn default_severity(&self) -> Severity {
        Severity::High
    }
    fn check(&self, ctx: &RuleContext<'_>) -> Vec<Diagnostic> {
        if ctx.file.kind != costguard_scanner::FileKind::Python {
            return Vec::new();
        }
        let regex =
            Regex::new(r#"(?i)\.apply\s*\([^\n]*axis\s*=\s*1|\.iterrows\s*\(|\.itertuples\s*\("#)
                .expect("valid python regex");
        regex
            .find_iter(&ctx.file.text)
            .map(|matched| {
                diagnostic(ctx, self.id(), self.default_severity(), Some(ctx.file.line_index.span(matched.start(), matched.end())), "Python model uses row-wise dataframe logic.")
                    .with_risk("row-wise Python logic is often much slower and more memory-intensive than vectorized logic.")
                    .with_suggestion("vectorize, use SQL, or use dataframe-native column operations.")
            })
            .collect()
    }
}

impl Rule for SourceInMartRule {
    fn id(&self) -> &'static str {
        "SQLCOST011"
    }
    fn name(&self) -> &'static str {
        "Source used directly in mart layer"
    }
    fn description(&self) -> &'static str {
        "Detects dbt source() usage in marts."
    }
    fn default_severity(&self) -> Severity {
        Severity::Medium
    }
    fn check(&self, ctx: &RuleContext<'_>) -> Vec<Diagnostic> {
        let Some(sql) = ctx.sql else {
            return Vec::new();
        };
        if !ctx
            .file
            .root_relative_path
            .to_string_lossy()
            .replace('\\', "/")
            .contains("marts/")
        {
            return Vec::new();
        }
        if sql.dbt.sources.is_empty() {
            return Vec::new();
        }
        vec![diagnostic(
            ctx,
            self.id(),
            self.default_severity(),
            None,
            "dbt source() is used directly in a mart model.",
        )
        .with_risk("mart models can inherit raw-source cost and data quality problems.")
        .with_suggestion("route raw sources through staging models first.")]
    }
}

impl Rule for CrossJoinRule {
    fn id(&self) -> &'static str {
        "SQLCOST012"
    }
    fn name(&self) -> &'static str {
        "Cross join without explicit allow comment"
    }
    fn description(&self) -> &'static str {
        "Detects CROSS JOIN and comma joins."
    }
    fn default_severity(&self) -> Severity {
        Severity::High
    }
    fn check(&self, ctx: &RuleContext<'_>) -> Vec<Diagnostic> {
        let Some(sql) = ctx.sql else {
            return Vec::new();
        };
        sql.features
            .joins
            .iter()
            .filter(|join| matches!(join.kind, JoinKind::Cross | JoinKind::Comma))
            .map(|join| {
                diagnostic(ctx, self.id(), self.default_severity(), Some(join.span), "Cross join detected without an explicit allow comment.")
                    .with_risk("cross joins can multiply row counts and create unexpectedly expensive queries.")
                    .with_suggestion("add a join predicate, or document intent with 'costguard: allow cross-join'.")
            })
            .collect()
    }
}

impl Rule for UnpartitionedWindowRule {
    fn id(&self) -> &'static str {
        "SQLCOST013"
    }
    fn name(&self) -> &'static str {
        "Unpartitioned window function"
    }
    fn description(&self) -> &'static str {
        "Detects OVER () and window functions without PARTITION BY."
    }
    fn default_severity(&self) -> Severity {
        Severity::Medium
    }
    fn check(&self, ctx: &RuleContext<'_>) -> Vec<Diagnostic> {
        let Some(sql) = ctx.sql else {
            return Vec::new();
        };
        sql.features
            .window_functions
            .iter()
            .filter(|window| !window.has_partition_by)
            .map(|window| {
                diagnostic(
                    ctx,
                    self.id(),
                    self.default_severity(),
                    Some(window.span),
                    "Window function has no PARTITION BY.",
                )
                .with_risk("unpartitioned windows can force broad sorts or scans.")
                .with_suggestion("partition by the natural entity key when possible.")
            })
            .collect()
    }
}

impl Rule for RepeatedCteRule {
    fn id(&self) -> &'static str {
        "SQLCOST014"
    }
    fn name(&self) -> &'static str {
        "Repeated CTE reference"
    }
    fn description(&self) -> &'static str {
        "Detects CTEs referenced multiple times downstream."
    }
    fn default_severity(&self) -> Severity {
        Severity::Low
    }
    fn check(&self, ctx: &RuleContext<'_>) -> Vec<Diagnostic> {
        let Some(sql) = ctx.sql else {
            return Vec::new();
        };
        let mut counts: HashMap<&str, usize> = HashMap::new();
        for reference in &sql.features.cte_references {
            *counts.entry(&reference.key).or_default() += 1;
        }
        sql.features
            .cte_references
            .iter()
            .find(|reference| counts.get(reference.key.as_str()).copied().unwrap_or(0) > 1)
            .map_or_else(Vec::new, |reference| {
                vec![diagnostic(
                    ctx,
                    self.id(),
                    self.default_severity(),
                    Some(reference.span),
                    "CTE is referenced multiple times.",
                )
                .with_risk("some warehouses may recompute expensive CTEs or produce larger plans.")
                .with_suggestion(
                    "materialize the CTE if it is expensive or reused across branches.",
                )]
            })
    }
}

impl Rule for RepeatedExpensiveAcrossFilesRule {
    fn id(&self) -> &'static str {
        "SQLCOST015"
    }
    fn name(&self) -> &'static str {
        "Expensive expression repeated across downstream models"
    }
    fn description(&self) -> &'static str {
        "Detects repeated JSON, regex, or normalization expressions across files."
    }
    fn default_severity(&self) -> Severity {
        Severity::Medium
    }
    fn check(&self, ctx: &RuleContext<'_>) -> Vec<Diagnostic> {
        let Some(sql) = ctx.sql else {
            return Vec::new();
        };
        let mut file_counts: HashMap<&str, usize> = HashMap::new();
        for doc in ctx.all_sql {
            let mut seen = std::collections::HashSet::new();
            for feature in doc
                .features
                .json_extractions
                .iter()
                .chain(doc.features.regex_calls.iter())
                .chain(doc.features.normalization_calls.iter())
            {
                seen.insert(feature.key.as_str());
            }
            for key in seen {
                *file_counts.entry(key).or_default() += 1;
            }
        }
        sql.features
            .json_extractions
            .iter()
            .chain(sql.features.regex_calls.iter())
            .chain(sql.features.normalization_calls.iter())
            .find(|feature| file_counts.get(feature.key.as_str()).copied().unwrap_or(0) > 1)
            .map_or_else(Vec::new, |feature| {
                vec![diagnostic(
                    ctx,
                    self.id(),
                    self.default_severity(),
                    Some(feature.span),
                    "Expensive expression appears in multiple files.",
                )
                .with_risk(
                    "recomputing expensive expressions downstream can compound warehouse cost.",
                )
                .with_suggestion("materialize the expression once upstream.")]
            })
    }
}

fn diagnostic(
    ctx: &RuleContext<'_>,
    rule_id: &str,
    severity: Severity,
    span: Option<costguard_diagnostics::Span>,
    message: &str,
) -> Diagnostic {
    Diagnostic::new(
        rule_id,
        severity,
        ctx.file.root_relative_path.clone(),
        span,
        message,
    )
    .with_warehouse(ctx.warehouse.to_string())
}

fn threshold(ctx: &RuleContext<'_>, rule_id: &str, default: usize) -> usize {
    ctx.overrides
        .get(rule_id)
        .and_then(|settings| settings.threshold)
        .unwrap_or(default)
}

fn is_downstream_model(path: &Path) -> bool {
    let path = path
        .to_string_lossy()
        .replace('\\', "/")
        .to_ascii_lowercase();
    path.contains("marts/")
        || path.contains("intermediate/")
        || path.contains("/int_")
        || path.contains("/fct_")
        || path.contains("/dim_")
}

fn is_staging_model(path: &Path) -> bool {
    let path = path
        .to_string_lossy()
        .replace('\\', "/")
        .to_ascii_lowercase();
    path.contains("staging/") || path.contains("/stg_")
}

fn incremental_predicate_suggestion(warehouse: Warehouse) -> &'static str {
    match warehouse {
        Warehouse::BigQuery => "add a partition predicate such as _PARTITIONDATE or an event_date filter.",
        Warehouse::Snowflake => "add an updated_at/event_date predicate to improve pruning on clustered or micro-partitioned data.",
        Warehouse::Databricks => "add a partition or date predicate to limit incremental reads.",
        _ => "add an updated_at, created_at, event_date, ingested_at, or partition predicate.",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use costguard_diagnostics::LineIndex;
    use costguard_scanner::{FileKind, ProjectFile};
    use costguard_sql::{analyze_sql, SqlDialect};
    use std::path::PathBuf;

    #[test]
    fn detects_select_star_in_mart() {
        let text = "select * from orders";
        let file = ProjectFile {
            path: PathBuf::from("models/marts/fct_orders.sql"),
            root_relative_path: PathBuf::from("models/marts/fct_orders.sql"),
            kind: FileKind::DbtSqlModel,
            text: text.into(),
            line_index: LineIndex::new(text),
        };
        let sql = analyze_sql(
            file.path.clone(),
            text,
            SqlDialect::Generic,
            &file.line_index,
        );
        let registry = RuleRegistry::default_rules();
        let diagnostics = registry.run(&RuleContext {
            warehouse: Warehouse::Generic,
            file: &file,
            sql: Some(&sql),
            dbt_model: None,
            all_sql: std::slice::from_ref(&sql),
            overrides: &RuleOverrides::default(),
        });
        assert!(diagnostics.iter().any(|d| d.rule_id == "SQLCOST001"));
    }
}
