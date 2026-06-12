use crate::helpers::{
    diagnostic, has_bounded_incremental_predicate, incremental_predicate_suggestion,
    incremental_source_filter_deferred, normalized_path,
};
use crate::registry::{Rule, RuleContext, Warehouse};
use costguard_dbt::DbtConfig;
use costguard_diagnostics::{Diagnostic, Severity};

pub(crate) struct IncrementalUniqueKeyRule;
pub(crate) struct IncrementalPredicateRule;
pub(crate) struct IncrementalSourceBoundRule;
pub(crate) struct SourceInMartRule;
pub(crate) struct MissingPartitionClusterRule;
pub(crate) struct FullRefreshHeavyIncrementalRule;

fn materialized_config<'a>(
    model: Option<&'a costguard_dbt::DbtModel>,
    sql_config: &'a DbtConfig,
) -> Option<&'a str> {
    model
        .and_then(|model| model.materialized.as_deref())
        .or(sql_config.materialized.as_deref())
}

fn partition_by_config<'a>(
    model: Option<&'a costguard_dbt::DbtModel>,
    sql_config: &'a DbtConfig,
) -> Option<&'a str> {
    model
        .and_then(|model| model.partition_by.as_deref())
        .or(sql_config.partition_by.as_deref())
}

fn cluster_by_config<'a>(
    model: Option<&'a costguard_dbt::DbtModel>,
    sql_config: &'a DbtConfig,
) -> Option<&'a str> {
    model
        .and_then(|model| model.cluster_by.as_deref())
        .or(sql_config.cluster_by.as_deref())
}

fn on_schema_change_config<'a>(
    model: Option<&'a costguard_dbt::DbtModel>,
    sql_config: &'a DbtConfig,
) -> Option<&'a str> {
    model
        .and_then(|model| model.on_schema_change.as_deref())
        .or(sql_config.on_schema_change.as_deref())
}

fn full_refresh_config(
    model: Option<&costguard_dbt::DbtModel>,
    sql_config: &DbtConfig,
) -> Option<bool> {
    model
        .and_then(|model| model.full_refresh)
        .or(sql_config.full_refresh)
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
        let materialized = ctx
            .dbt_model
            .and_then(|model| model.materialized.as_deref())
            .or(sql.dbt.config.materialized.as_deref());
        let unique_key = ctx
            .dbt_model
            .and_then(|model| model.unique_key.as_deref())
            .or(sql.dbt.config.unique_key.as_deref());
        let incremental_strategy = ctx
            .dbt_model
            .and_then(|model| model.incremental_strategy.as_deref())
            .or(sql.dbt.config.incremental_strategy.as_deref());
        if materialized == Some("incremental") && unique_key.is_none() {
            if incremental_strategy.is_some_and(|strategy| strategy.eq_ignore_ascii_case("append"))
            {
                return Vec::new();
            }
            vec![diagnostic(
                ctx,
                self.id(),
                self.default_severity(),
                None,
                "Incremental model appears to have no unique_key.",
            )
            .with_risk("merge/update logic may be unsafe or require full scans.")
            .with_suggestion(
                "define config(unique_key=...) or set incremental_strategy='append' for append-only models.",
            )]
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
        let materialized = ctx
            .dbt_model
            .and_then(|model| model.materialized.as_deref())
            .or(sql.dbt.config.materialized.as_deref());
        if materialized != Some("incremental") || !sql.dbt.uses_is_incremental {
            return Vec::new();
        }
        if has_bounded_incremental_predicate(&ctx.file.text) {
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

impl Rule for IncrementalSourceBoundRule {
    fn id(&self) -> &'static str {
        "SQLCOST019"
    }
    fn name(&self) -> &'static str {
        "Incremental model reads source without source-side bound"
    }
    fn description(&self) -> &'static str {
        "Detects incremental models that read source() before applying a partition predicate."
    }
    fn default_severity(&self) -> Severity {
        Severity::High
    }
    fn check(&self, ctx: &RuleContext<'_>) -> Vec<Diagnostic> {
        let Some(sql) = ctx.sql else {
            return Vec::new();
        };
        let materialized = ctx
            .dbt_model
            .and_then(|model| model.materialized.as_deref())
            .or(sql.dbt.config.materialized.as_deref());
        if !incremental_source_filter_deferred(
            &ctx.file.text,
            sql.dbt.uses_is_incremental,
            !sql.dbt.sources.is_empty(),
            materialized,
            sql.dbt.incremental_block.as_deref(),
        ) {
            return Vec::new();
        }
        vec![diagnostic(
            ctx,
            self.id(),
            self.default_severity(),
            None,
            "Incremental model reads source() before applying a partition or date predicate.",
        )
        .with_risk(
            "incremental runs may scan far more raw source data than intended even when a downstream filter exists.",
        )
        .with_suggestion(
            "push the partition or date predicate into the first CTE or subquery that reads source().",
        )]
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
        if !normalized_path(&ctx.file.root_relative_path).contains("marts/")
            || sql.dbt.sources.is_empty()
        {
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

impl Rule for MissingPartitionClusterRule {
    fn id(&self) -> &'static str {
        "SQLCOST028"
    }
    fn name(&self) -> &'static str {
        "Missing partition or cluster config on large mart model"
    }
    fn description(&self) -> &'static str {
        "Detects incremental or table materialized mart models without partition_by or cluster_by."
    }
    fn default_severity(&self) -> Severity {
        Severity::High
    }
    fn applies_to(&self, warehouse: Warehouse) -> bool {
        matches!(
            warehouse,
            Warehouse::BigQuery | Warehouse::Snowflake | Warehouse::Databricks
        )
    }
    fn check(&self, ctx: &RuleContext<'_>) -> Vec<Diagnostic> {
        let path = normalized_path(&ctx.file.root_relative_path);
        if !path.contains("marts/") && !path.contains("mart/") {
            return Vec::new();
        }
        let Some(sql) = ctx.sql else {
            return Vec::new();
        };
        let materialized = materialized_config(ctx.dbt_model, &sql.dbt.config);
        let Some(materialized) = materialized else {
            return Vec::new();
        };
        if materialized != "incremental" && materialized != "table" {
            return Vec::new();
        }
        let has_partition = partition_by_config(ctx.dbt_model, &sql.dbt.config).is_some();
        let has_cluster = cluster_by_config(ctx.dbt_model, &sql.dbt.config).is_some();
        if has_partition || has_cluster {
            return Vec::new();
        }
        vec![diagnostic(
            ctx,
            self.id(),
            self.default_severity(),
            None,
            "Mart model has no partition_by or cluster_by configuration.",
        )
        .with_risk(
            "large mart tables without partition or cluster hints can scan and shuffle far more data than intended.",
        )
        .with_suggestion(
            "add partition_by and/or cluster_by in model config for warehouse-native pruning.",
        )]
    }
}

impl Rule for FullRefreshHeavyIncrementalRule {
    fn id(&self) -> &'static str {
        "SQLCOST029"
    }
    fn name(&self) -> &'static str {
        "Full-refresh-heavy incremental config"
    }
    fn description(&self) -> &'static str {
        "Detects incremental models configured with full_refresh or sync_all_columns schema changes."
    }
    fn default_severity(&self) -> Severity {
        Severity::Medium
    }
    fn check(&self, ctx: &RuleContext<'_>) -> Vec<Diagnostic> {
        let Some(sql) = ctx.sql else {
            return Vec::new();
        };
        let materialized = materialized_config(ctx.dbt_model, &sql.dbt.config);
        if materialized != Some("incremental") {
            return Vec::new();
        }
        let full_refresh = full_refresh_config(ctx.dbt_model, &sql.dbt.config).unwrap_or(false);
        let on_schema_change = on_schema_change_config(ctx.dbt_model, &sql.dbt.config);
        let sync_all_columns =
            on_schema_change.is_some_and(|value| value.eq_ignore_ascii_case("sync_all_columns"));
        if !full_refresh && !sync_all_columns {
            return Vec::new();
        }
        let message = if full_refresh {
            "Incremental model is configured with full_refresh=true."
        } else {
            "Incremental model uses on_schema_change=sync_all_columns."
        };
        vec![diagnostic(ctx, self.id(), self.default_severity(), None, message)
            .with_risk(
                "these settings can force expensive full rebuilds or wide schema rewrites on incremental models.",
            )
            .with_suggestion(
                "prefer bounded incremental merges; use append_new_columns or fail schema changes in production marts.",
            )]
    }
}
