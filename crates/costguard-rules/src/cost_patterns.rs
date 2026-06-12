use crate::helpers::{diagnostic, is_downstream_model, is_staging_model};
use crate::registry::{Rule, RuleContext, Warehouse};
use costguard_diagnostics::{Confidence, Diagnostic, Severity};
use costguard_sql::JoinKind;

pub(crate) struct NonSargablePredicateRule;
pub(crate) struct FunctionWrappedJoinKeyRule;
pub(crate) struct UnionWithoutAllRule;
pub(crate) struct CountDistinctRule;
pub(crate) struct WildcardTableScanRule;
pub(crate) struct CorrelatedSubqueryRule;
pub(crate) struct LeadingWildcardLikeRule;
pub(crate) struct OrPartitionPredicateRule;
pub(crate) struct PatternMatchingJoinRule;
pub(crate) struct ScalarSubqueryInSelectRule;
pub(crate) struct CrossCatalogJoinRule;

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

impl Rule for CorrelatedSubqueryRule {
    fn id(&self) -> &'static str {
        "SQLCOST030"
    }
    fn name(&self) -> &'static str {
        "Correlated subquery"
    }
    fn description(&self) -> &'static str {
        "Detects correlated subqueries in filters or join predicates."
    }
    fn default_severity(&self) -> Severity {
        Severity::High
    }
    fn check(&self, ctx: &RuleContext<'_>) -> Vec<Diagnostic> {
        let Some(sql) = ctx.sql else {
            return Vec::new();
        };
        sql.features
            .correlated_subqueries
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
                    "Correlated subquery detected.",
                )
                .with_risk(
                    "correlated subqueries often execute once per outer row and can dominate warehouse cost.",
                )
                .with_suggestion(
                    "rewrite as a join or pre-aggregate/filter the driving set before the subquery.",
                )
                .with_confidence(confidence)
            })
            .collect()
    }
}

impl Rule for LeadingWildcardLikeRule {
    fn id(&self) -> &'static str {
        "SQLCOST031"
    }
    fn name(&self) -> &'static str {
        "Leading-wildcard LIKE predicate"
    }
    fn description(&self) -> &'static str {
        "Detects LIKE/ILIKE patterns that start with % or _ in filters."
    }
    fn default_severity(&self) -> Severity {
        Severity::Medium
    }
    fn check(&self, ctx: &RuleContext<'_>) -> Vec<Diagnostic> {
        let Some(sql) = ctx.sql else {
            return Vec::new();
        };
        sql.features
            .leading_wildcard_likes
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
                    "LIKE predicate starts with a wildcard.",
                )
                .with_risk("leading wildcards usually prevent index or partition pruning on the filtered column.")
                .with_suggestion(
                    "use prefix search, tokenized lookup tables, or search indexes instead of '%value' patterns.",
                )
                .with_confidence(confidence)
            })
            .collect()
    }
}

impl Rule for OrPartitionPredicateRule {
    fn id(&self) -> &'static str {
        "SQLCOST032"
    }
    fn name(&self) -> &'static str {
        "OR across partition or date predicates"
    }
    fn description(&self) -> &'static str {
        "Detects OR expressions joining predicates on likely partition or date columns."
    }
    fn default_severity(&self) -> Severity {
        Severity::Medium
    }
    fn check(&self, ctx: &RuleContext<'_>) -> Vec<Diagnostic> {
        let Some(sql) = ctx.sql else {
            return Vec::new();
        };
        sql.features
            .or_partition_predicates
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
                    "Filter ORs together partition or date predicates.",
                )
                .with_risk(
                    "OR across partition keys often forces multiple partition scans or full-table reads.",
                )
                .with_suggestion(
                    "split into UNION ALL branches or rewrite with IN lists on a single partition column.",
                )
                .with_confidence(confidence)
            })
            .collect()
    }
}

impl Rule for PatternMatchingJoinRule {
    fn id(&self) -> &'static str {
        "SQLCOST033"
    }
    fn name(&self) -> &'static str {
        "Pattern-matching join predicate"
    }
    fn description(&self) -> &'static str {
        "Detects LIKE, RLIKE, or regexp_like predicates in JOIN ON clauses."
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
            .filter(|join| join.pattern_matching)
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
                    "Join predicate uses pattern matching.",
                )
                .with_risk(
                    "pattern joins often devolve into nested loops or cartesian explosions at scale.",
                )
                .with_suggestion(
                    "normalize join keys upstream or use equality joins on tokenized columns.",
                )
                .with_confidence(confidence)
            })
            .collect()
    }
}

impl Rule for ScalarSubqueryInSelectRule {
    fn id(&self) -> &'static str {
        "SQLCOST034"
    }
    fn name(&self) -> &'static str {
        "Scalar subquery in SELECT list"
    }
    fn description(&self) -> &'static str {
        "Detects per-row scalar subqueries in downstream model projections."
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
        sql.features
            .scalar_subqueries_in_select
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
                    "Scalar subquery appears in the SELECT list.",
                )
                .with_risk(
                    "per-row scalar subqueries can execute once for every output row in downstream models.",
                )
                .with_suggestion("join or pre-aggregate the lookup once, then select the joined column.")
                .with_confidence(confidence)
            })
            .collect()
    }
}

impl Rule for CrossCatalogJoinRule {
    fn id(&self) -> &'static str {
        "SQLCOST035"
    }
    fn name(&self) -> &'static str {
        "Cross-catalog join"
    }
    fn description(&self) -> &'static str {
        "Detects joins between fully qualified tables with different catalog parts."
    }
    fn default_severity(&self) -> Severity {
        Severity::Medium
    }
    fn applies_to(&self, warehouse: Warehouse) -> bool {
        matches!(warehouse, Warehouse::Trino | Warehouse::Databricks)
    }
    fn check(&self, ctx: &RuleContext<'_>) -> Vec<Diagnostic> {
        let Some(sql) = ctx.sql else {
            return Vec::new();
        };
        sql.features
            .joins
            .iter()
            .filter(|join| join.cross_catalog)
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
                    "Join crosses catalogs in fully qualified table names.",
                )
                .with_risk(
                    "cross-catalog joins can force remote reads, extra coordinator work, and brittle query plans.",
                )
                .with_suggestion(
                    "stage remote catalog data locally or join within one catalog when possible.",
                )
                .with_confidence(confidence)
            })
            .collect()
    }
}
