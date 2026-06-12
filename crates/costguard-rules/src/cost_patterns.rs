use crate::helpers::{diagnostic, is_downstream_model, is_staging_model};
use crate::registry::{Rule, RuleContext, Warehouse};
use costguard_diagnostics::{Confidence, Diagnostic, Severity};
use costguard_sql::JoinKind;

pub(crate) struct NonSargablePredicateRule;
pub(crate) struct FunctionWrappedJoinKeyRule;
pub(crate) struct UnionWithoutAllRule;
pub(crate) struct CountDistinctRule;
pub(crate) struct WildcardTableScanRule;

impl Rule for NonSargablePredicateRule {
    fn id(&self) -> &'static str {
        "SQLCOST016"
    }
    fn name(&self) -> &'static str {
        "Non-sargable partition or date predicate"
    }
    fn description(&self) -> &'static str {
        "Detects filters that wrap likely partition or date columns in functions."
    }
    fn default_severity(&self) -> Severity {
        Severity::High
    }
    fn check(&self, ctx: &RuleContext<'_>) -> Vec<Diagnostic> {
        let Some(sql) = ctx.sql else {
            return Vec::new();
        };
        if is_staging_model(&ctx.file.root_relative_path) {
            return Vec::new();
        }
        sql.features
            .non_sargable_predicates
            .iter()
            .map(|feature| {
                let confidence = if sql.feature_extraction_used_ast {
                    Confidence::High
                } else {
                    Confidence::Low
                };
                diagnostic(
                    ctx,
                    self.id(),
                    self.default_severity(),
                    Some(feature.span),
                    "Partition or date column is wrapped in a function inside a filter.",
                )
                .with_risk(
                    "warehouses often cannot prune partitions efficiently when the column is wrapped.",
                )
                .with_suggestion(
                    "compare the raw column to a bounded range, e.g. block_time >= current_date and block_time < current_date + interval '1 day'.",
                )
                .with_confidence(confidence)
            })
            .collect()
    }
}

impl Rule for FunctionWrappedJoinKeyRule {
    fn id(&self) -> &'static str {
        "SQLCOST017"
    }
    fn name(&self) -> &'static str {
        "Function-wrapped join key"
    }
    fn description(&self) -> &'static str {
        "Detects joins where a join key is transformed inline."
    }
    fn default_severity(&self) -> Severity {
        Severity::Medium
    }
    fn check(&self, ctx: &RuleContext<'_>) -> Vec<Diagnostic> {
        let Some(sql) = ctx.sql else {
            return Vec::new();
        };
        if is_staging_model(&ctx.file.root_relative_path) {
            return Vec::new();
        }
        sql.features
            .joins
            .iter()
            .filter(|join| {
                matches!(
                    join.kind,
                    JoinKind::Inner | JoinKind::Left | JoinKind::Right | JoinKind::Full
                ) && join.has_equality
                    && join.function_on_join_key
            })
            .map(|join| {
                let confidence = if sql.feature_extraction_used_ast {
                    Confidence::High
                } else {
                    Confidence::Low
                };
                diagnostic(
                    ctx,
                    self.id(),
                    self.default_severity(),
                    Some(join.span),
                    "Join equality uses a function-wrapped key.",
                )
                .with_risk(
                    "function-wrapped joins often block optimizations, increase shuffle cost, and hide upstream modeling problems.",
                )
                .with_suggestion(
                    "normalize keys once in staging, then join on stored normalized columns.",
                )
                .with_confidence(confidence)
            })
            .collect()
    }
}

impl Rule for UnionWithoutAllRule {
    fn id(&self) -> &'static str {
        "SQLCOST018"
    }
    fn name(&self) -> &'static str {
        "UNION instead of UNION ALL"
    }
    fn description(&self) -> &'static str {
        "Detects plain UNION in dbt models."
    }
    fn default_severity(&self) -> Severity {
        Severity::Medium
    }
    fn check(&self, ctx: &RuleContext<'_>) -> Vec<Diagnostic> {
        let Some(sql) = ctx.sql else {
            return Vec::new();
        };
        let severity = if is_staging_model(&ctx.file.root_relative_path)
            || ctx
                .file
                .root_relative_path
                .to_string_lossy()
                .to_ascii_lowercase()
                .contains("intermediate/")
        {
            Severity::High
        } else {
            self.default_severity()
        };
        sql.features
            .unions_without_all
            .iter()
            .map(|feature| {
                let confidence = if sql.feature_extraction_used_ast {
                    Confidence::High
                } else {
                    Confidence::Low
                };
                diagnostic(
                    ctx,
                    self.id(),
                    severity,
                    Some(feature.span),
                    "Plain UNION detected; UNION ALL is usually cheaper for dbt model stacking.",
                )
                .with_risk(
                    "UNION deduplicates and can force expensive global sorts or hash aggregation.",
                )
                .with_suggestion("use UNION ALL when duplicate rows are not expected.")
                .with_confidence(confidence)
            })
            .collect()
    }
}

impl Rule for CountDistinctRule {
    fn id(&self) -> &'static str {
        "SQLCOST020"
    }
    fn name(&self) -> &'static str {
        "Exact distinct aggregation on large model"
    }
    fn description(&self) -> &'static str {
        "Detects count(distinct ...) in downstream models."
    }
    fn default_severity(&self) -> Severity {
        Severity::Medium
    }
    fn check(&self, ctx: &RuleContext<'_>) -> Vec<Diagnostic> {
        let Some(sql) = ctx.sql else {
            return Vec::new();
        };
        if !is_downstream_model(&ctx.file.root_relative_path) {
            return Vec::new();
        }
        let severity = if sql.features.count_distincts.len() > 1 {
            Severity::High
        } else {
            self.default_severity()
        };
        sql.features
            .count_distincts
            .iter()
            .map(|feature| {
                let confidence = if sql.feature_extraction_used_ast {
                    Confidence::High
                } else {
                    Confidence::Low
                };
                diagnostic(
                    ctx,
                    self.id(),
                    severity,
                    Some(feature.span),
                    "Exact COUNT(DISTINCT ...) detected.",
                )
                .with_risk(
                    "exact distinct aggregation can be one of the most expensive operations at warehouse scale.",
                )
                .with_suggestion(
                    "pre-aggregate, dedupe upstream, or use approximate distinct where acceptable.",
                )
                .with_confidence(confidence)
            })
            .collect()
    }
}

impl Rule for WildcardTableScanRule {
    fn id(&self) -> &'static str {
        "SQLCOST021"
    }
    fn name(&self) -> &'static str {
        "BigQuery wildcard table scan without suffix bound"
    }
    fn description(&self) -> &'static str {
        "Detects wildcard tables without a bounded _TABLE_SUFFIX filter."
    }
    fn default_severity(&self) -> Severity {
        Severity::Medium
    }
    fn applies_to(&self, warehouse: Warehouse) -> bool {
        warehouse == Warehouse::BigQuery
    }
    fn check(&self, ctx: &RuleContext<'_>) -> Vec<Diagnostic> {
        let Some(sql) = ctx.sql else {
            return Vec::new();
        };
        sql.features
            .wildcard_table_scans
            .iter()
            .map(|feature| {
                let confidence = if sql.feature_extraction_used_ast {
                    Confidence::High
                } else {
                    Confidence::Low
                };
                diagnostic(
                    ctx,
                    self.id(),
                    self.default_severity(),
                    Some(feature.span),
                    "Wildcard table reference without an obvious _TABLE_SUFFIX bound.",
                )
                .with_risk(
                    "BigQuery wildcard scans can silently read far more tables than intended.",
                )
                .with_suggestion(
                    "add a _TABLE_SUFFIX between filter or otherwise bound the wildcard scan.",
                )
                .with_confidence(confidence)
            })
            .collect()
    }
}
