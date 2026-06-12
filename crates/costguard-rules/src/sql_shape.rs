use crate::helpers::{diagnostic, is_downstream_model, is_staging_model};
use crate::registry::{Rule, RuleContext};
use costguard_diagnostics::{Confidence, Diagnostic, Severity};
use costguard_sql::JoinKind;
use std::collections::HashMap;

pub(crate) struct SelectStarRule;
pub(crate) struct UnboundedJoinRule;
pub(crate) struct OrderByIntermediateRule;
pub(crate) struct BlindDistinctRule;
pub(crate) struct CrossJoinRule;
pub(crate) struct UnpartitionedWindowRule;
pub(crate) struct RepeatedCteRule;

fn join_has_clear_equality(join: &costguard_sql::JoinFeature) -> bool {
    join.has_equality
        || join
            .predicate
            .as_ref()
            .is_some_and(|predicate| predicate.contains('='))
}

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
                ) && !join_has_clear_equality(join)
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
                    "non-equality joins can cause large intermediate scans and unexpected row multiplication.",
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
        if ctx.file.text.to_ascii_lowercase().contains("group by") {
            return Vec::new();
        }
        sql.features
            .distincts
            .iter()
            .map(|feature| {
                diagnostic(
                    ctx,
                    self.id(),
                    self.default_severity(),
                    Some(feature.span),
                    "SELECT DISTINCT detected.",
                )
                .with_risk(
                    "blind deduplication can hide upstream data quality issues and force large sorts.",
                )
                .with_suggestion(
                    "deduplicate explicitly with keys and row_number() when that is the intent.",
                )
            })
            .collect()
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
        Severity::Medium
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
                    "Cross join detected without an explicit allow comment.",
                )
                .with_risk(
                    "cross joins can multiply row counts and create unexpectedly expensive queries.",
                )
                .with_suggestion(
                    "add a join predicate, or document intent with 'costguard: allow cross-join'.",
                )
                .with_confidence(confidence)
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
            .find(|reference| counts.get(reference.key.as_str()).copied().unwrap_or(0) >= 3)
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
