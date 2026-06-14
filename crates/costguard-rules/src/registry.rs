use crate::evidence;
use costguard_dbt::DbtModel;
use costguard_diagnostics::{Diagnostic, Severity};
use costguard_scanner::ProjectFile;
use costguard_sql::SqlDocument;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};

pub use costguard_platform::Platform;

/// Per-rule override from configuration (enable, severity, threshold).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuleOverride {
    pub enabled: Option<bool>,
    pub severity: Option<Severity>,
    pub threshold: Option<usize>,
}

/// Map of rule ID to [`RuleOverride`] from `costguard.toml`.
pub type RuleOverrides = HashMap<String, RuleOverride>;

/// Cross-file indexes built once per scan for rules that need project-wide context.
#[derive(Debug, Clone, Default)]
pub struct ModelMeta {
    pub materialized: Option<String>,
    pub unique_key: Option<Vec<String>>,
}

/// Cross-file indexes built once per scan for rules that need project-wide context.
#[derive(Debug, Clone, Default)]
pub struct ProjectIndexes {
    pub expensive_expression_file_counts: HashMap<String, usize>,
    pub model_ref_counts: HashMap<String, usize>,
    pub model_meta: HashMap<String, ModelMeta>,
}

impl ProjectIndexes {
    pub fn from_sql_documents(sql_documents: &[SqlDocument]) -> Self {
        let mut file_counts = HashMap::new();
        let mut model_ref_counts = HashMap::new();
        let mut model_meta = HashMap::new();
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
            for dbt_ref in &doc.dbt.refs {
                *model_ref_counts.entry(dbt_ref.name.clone()).or_default() += 1;
            }
            if let Some(model_name) = doc
                .path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .map(str::to_string)
            {
                let unique_key = doc
                    .dbt
                    .config
                    .unique_key
                    .as_deref()
                    .map(parse_unique_key_columns);
                model_meta.entry(model_name).or_insert_with(|| ModelMeta {
                    materialized: doc.dbt.config.materialized.clone(),
                    unique_key,
                });
            }
        }
        Self {
            expensive_expression_file_counts: file_counts,
            model_ref_counts,
            model_meta,
        }
    }
}

fn parse_unique_key_columns(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(|part| part.trim().trim_matches(['\'', '"']).to_ascii_lowercase())
        .filter(|part| !part.is_empty())
        .collect()
}

/// Input context passed to each rule during evaluation.
pub struct RuleContext<'a> {
    pub warehouse: Platform,
    pub file: &'a ProjectFile,
    pub sql: Option<&'a SqlDocument>,
    pub dbt_model: Option<&'a DbtModel>,
    pub all_sql: &'a [SqlDocument],
    pub project_indexes: &'a ProjectIndexes,
    pub overrides: &'a RuleOverrides,
}

/// A single SQLCOST rule that inspects a [`RuleContext`] and returns diagnostics.
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

/// Static metadata for a registered rule (id, name, description, default severity).
#[derive(Debug, Clone, Serialize)]
pub struct RuleMetadata {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub severity: Severity,
}

/// Registry of all built-in SQLCOST rules.
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
                Box::new(crate::cost_patterns::RowExplosionRule),
                Box::new(crate::cost_patterns::NotInSubqueryRule),
                Box::new(crate::cross_file::FanOutJoinRule),
                Box::new(crate::cross_file::UnmaterializedHeavyViewRule),
                Box::new(crate::dbt::TableShouldBeIncrementalRule),
                Box::new(crate::cost_patterns::UnboundedWindowFrameRule),
                Box::new(crate::cost_patterns::BigQueryMissingPartitionFilterRule),
                Box::new(crate::dbt::MergeWithoutTargetPruningRule),
                Box::new(crate::sql_shape::RecursiveCteRule),
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
            .flat_map(|rule| rule.check(ctx))
            .collect()
    }

    pub fn run_declarative(
        &self,
        ctx: &RuleContext<'_>,
        rules: &[costguard_policy::DeclarativeRule],
    ) -> anyhow::Result<Vec<Diagnostic>> {
        let facts = rule_facts(ctx);
        let mut diagnostics = Vec::new();
        for rule in rules {
            if costguard_policy::evaluate_custom_rule(rule, &facts)? {
                let mut diagnostic = Diagnostic::new(
                    rule.id.clone(),
                    rule.severity,
                    ctx.file.root_relative_path.clone(),
                    None,
                    rule.message.clone(),
                )
                .with_confidence(rule.confidence)
                .with_warehouse(ctx.warehouse.to_string());
                if let Some(risk) = &rule.risk {
                    diagnostic = diagnostic.with_risk(risk.clone());
                }
                if let Some(suggestion) = &rule.suggestion {
                    diagnostic = diagnostic.with_suggestion(suggestion.clone());
                }
                let predicate_hash = costguard_policy::canonical_json(&rule.predicate)?;
                diagnostic.governance.evidence_key =
                    evidence::declarative(&rule.id, &predicate_hash);
                diagnostics.push(diagnostic);
            }
        }
        Ok(diagnostics)
    }
}

fn rule_facts(ctx: &RuleContext<'_>) -> costguard_policy::RuleFactsV1 {
    let mut facts = costguard_policy::RuleFactsV1::default();
    facts
        .strings
        .insert("platform".into(), ctx.warehouse.to_string());
    facts.strings.insert(
        "path".into(),
        ctx.file
            .root_relative_path
            .to_string_lossy()
            .replace('\\', "/"),
    );
    facts.strings.insert(
        "file.kind".into(),
        format!("{:?}", ctx.file.kind).to_ascii_lowercase(),
    );
    facts.strings.insert(
        "metadata.state".into(),
        if ctx.dbt_model.is_some() {
            "dbt_model"
        } else {
            "source_only"
        }
        .into(),
    );
    if let Some(model) = ctx.dbt_model {
        if let Some(materialized) = &model.materialized {
            facts
                .strings
                .insert("dbt.materialized".into(), materialized.clone());
        }
        facts.strings.insert("dbt.model".into(), model.name.clone());
        for (field, value) in [
            ("dbt.unique_key", model.unique_key.as_ref()),
            (
                "dbt.incremental_strategy",
                model.incremental_strategy.as_ref(),
            ),
            ("dbt.partition_by", model.partition_by.as_ref()),
            ("dbt.cluster_by", model.cluster_by.as_ref()),
            ("dbt.on_schema_change", model.on_schema_change.as_ref()),
        ] {
            if let Some(value) = value {
                facts.strings.insert(field.into(), value.clone());
            }
        }
        facts.sets.insert(
            "dbt.tags".into(),
            model.tags.iter().cloned().collect::<BTreeSet<_>>(),
        );
        facts.sets.insert(
            "lineage.refs".into(),
            model.refs.iter().cloned().collect::<BTreeSet<_>>(),
        );
        facts.sets.insert(
            "lineage.sources".into(),
            model
                .sources
                .iter()
                .map(|source| format!("{}.{}", source.source_name, source.table_name))
                .collect::<BTreeSet<_>>(),
        );
        facts
            .numbers
            .insert("dbt.ref_count".into(), model.refs.len() as f64);
        facts
            .numbers
            .insert("dbt.source_count".into(), model.sources.len() as f64);
    }
    if let Some(sql) = ctx.sql {
        let features = &sql.features;
        for (name, value) in [
            ("sql.join_count", features.joins.len()),
            ("sql.window_count", features.window_functions.len()),
            ("sql.cte_count", features.ctes.len()),
            ("sql.select_star_count", features.select_stars.len()),
            ("sql.json_extraction_count", features.json_extractions.len()),
            ("sql.regex_count", features.regex_calls.len()),
            (
                "sql.normalization_count",
                features.normalization_calls.len(),
            ),
            (
                "sql.non_sargable_count",
                features.non_sargable_predicates.len(),
            ),
            (
                "sql.correlated_subquery_count",
                features.correlated_subqueries.len(),
            ),
        ] {
            facts.numbers.insert(name.into(), value as f64);
        }
        facts.numbers.insert(
            "sql.cross_join_count".into(),
            features
                .joins
                .iter()
                .filter(|join| {
                    matches!(
                        join.kind,
                        costguard_sql::JoinKind::Cross | costguard_sql::JoinKind::Comma
                    )
                })
                .count() as f64,
        );
        facts.numbers.insert(
            "sql.equality_join_count".into(),
            features
                .joins
                .iter()
                .filter(|join| join.has_equality)
                .count() as f64,
        );
        facts.numbers.insert(
            "sql.unbounded_join_count".into(),
            features
                .joins
                .iter()
                .filter(|join| !join.has_equality && join.predicate.is_none())
                .count() as f64,
        );
        facts.numbers.insert(
            "sql.unpartitioned_window_count".into(),
            features
                .window_functions
                .iter()
                .filter(|window| !window.has_partition_by)
                .count() as f64,
        );
        facts.sets.insert(
            "sql.join_kinds".into(),
            features
                .joins
                .iter()
                .map(|join| format!("{:?}", join.kind).to_ascii_lowercase())
                .collect(),
        );
        let mut function_classes = BTreeSet::new();
        for (class, present) in [
            ("json", !features.json_extractions.is_empty()),
            ("regex", !features.regex_calls.is_empty()),
            ("normalization", !features.normalization_calls.is_empty()),
            ("window", !features.window_functions.is_empty()),
        ] {
            if present {
                function_classes.insert(class.into());
            }
        }
        facts
            .sets
            .insert("sql.function_classes".into(), function_classes);
        facts
            .strings
            .insert("sql.parsed".into(), sql.parsed.to_string());
    }
    facts
}
