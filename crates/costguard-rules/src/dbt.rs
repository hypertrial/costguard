use crate::helpers::{
    diagnostic, has_bounded_incremental_predicate, incremental_predicate_suggestion,
    normalized_path,
};
use crate::registry::{Rule, RuleContext};
use costguard_diagnostics::{Diagnostic, Severity};

pub(crate) struct IncrementalUniqueKeyRule;
pub(crate) struct IncrementalPredicateRule;
pub(crate) struct SourceInMartRule;

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
        let predicate_scope = sql
            .dbt
            .incremental_block
            .as_deref()
            .unwrap_or(&ctx.file.text);
        if has_bounded_incremental_predicate(predicate_scope) {
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
