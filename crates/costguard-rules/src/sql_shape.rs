use crate::evidence;
use crate::helpers::{diagnostic, is_dbt_macro_path, is_downstream_model, is_staging_model};
use crate::registry::{Rule, RuleContext};
use costguard_diagnostics::{Confidence, Diagnostic, Severity};
use costguard_sql::{CteFeature, JoinFeature, JoinKind};
use std::collections::HashMap;

pub(crate) struct SelectStarRule;
pub(crate) struct UnboundedJoinRule;
pub(crate) struct OrderByIntermediateRule;
pub(crate) struct BlindDistinctRule;
pub(crate) struct CrossJoinRule;
pub(crate) struct UnpartitionedWindowRule;
pub(crate) struct RepeatedCteRule;
pub(crate) struct RecursiveCteRule;

fn join_has_clear_equality(join: &JoinFeature, file_text: &str) -> bool {
    if join.has_equality || !join.equality_keys.is_empty() {
        return true;
    }
    if join.predicate.as_ref().is_some_and(|predicate| {
        let lower = predicate.to_ascii_lowercase();
        lower.contains('=') || lower.starts_with("using(")
    }) {
        return true;
    }
    join_on_clause_has_equality(file_text, join.span.byte_start)
}

fn join_on_clause_has_equality(text: &str, join_start: usize) -> bool {
    if on_clause_after_join_target_has_equality(text, join_start) {
        return true;
    }
    let start = join_start.min(text.len());
    let end = (start + 3000).min(text.len());
    on_snippet_has_equality(&text[start..end])
}

fn on_clause_after_join_target_has_equality(text: &str, join_start: usize) -> bool {
    let Some(after_join) = text.get(join_start..) else {
        return false;
    };
    let lower = after_join.to_ascii_lowercase();
    let rel_start = lower.find(" join").map(|idx| idx + 5).unwrap_or(0);
    let mut idx = skip_ascii_whitespace(after_join, rel_start);
    idx = skip_join_relation(after_join, idx);
    idx = skip_ascii_whitespace(after_join, idx);
    idx = skip_optional_alias(after_join, idx);
    let window_end = (idx + 240).min(after_join.len());
    on_snippet_has_equality(&after_join[idx..window_end])
}

fn skip_ascii_whitespace(text: &str, mut idx: usize) -> usize {
    while idx < text.len() && text.as_bytes()[idx].is_ascii_whitespace() {
        idx += 1;
    }
    idx
}

fn skip_join_relation(text: &str, idx: usize) -> usize {
    if text.as_bytes().get(idx) == Some(&b'(') {
        return skip_balanced_parens(text, idx);
    }
    if text.get(idx..).is_some_and(|tail| tail.starts_with("{{")) {
        return skip_jinja_expression(text, idx);
    }
    let mut end = idx;
    while end < text.len() {
        let byte = text.as_bytes()[end];
        if byte.is_ascii_whitespace() || byte == b'(' {
            break;
        }
        end += 1;
    }
    end
}

fn skip_balanced_parens(text: &str, idx: usize) -> usize {
    let mut depth = 0usize;
    let mut pos = idx;
    while pos < text.len() {
        match text.as_bytes()[pos] {
            b'(' => depth += 1,
            b')' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return pos + 1;
                }
            }
            _ => {}
        }
        pos += 1;
    }
    text.len()
}

fn skip_jinja_expression(text: &str, idx: usize) -> usize {
    let Some(tail) = text.get(idx..) else {
        return idx;
    };
    let mut depth = 0usize;
    for (offset, ch) in tail.char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return idx + offset + 1;
                }
            }
            _ => {}
        }
    }
    text.len()
}

fn skip_optional_alias(text: &str, idx: usize) -> usize {
    let mut pos = skip_ascii_whitespace(text, idx);
    if pos >= text.len() {
        return pos;
    }
    let lower_tail = text[pos..].to_ascii_lowercase();
    if lower_tail.starts_with("on ")
        || lower_tail.starts_with("using")
        || lower_tail.starts_with("join ")
    {
        return pos;
    }
    if text.as_bytes()[pos] == b'(' || text.get(pos..).is_some_and(|tail| tail.starts_with("{{")) {
        return pos;
    }
    while pos < text.len() {
        let byte = text.as_bytes()[pos];
        if byte.is_ascii_whitespace() {
            break;
        }
        pos += 1;
    }
    pos
}

fn on_snippet_has_equality(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    for (idx, _) in lower.match_indices(" on ") {
        let after = &lower[idx..];
        if after.starts_with(" on only ") {
            continue;
        }
        if after.starts_with(" using") {
            return true;
        }
        if let Some(eq_pos) = after.find('=') {
            let before = &after[..eq_pos];
            if !before.contains("<=")
                && !before.contains(">=")
                && !before.contains("!=")
                && !before.contains("<>")
            {
                return true;
            }
        }
    }
    false
}

fn is_cte_broadcast_cross_join(join: &JoinFeature, ctes: &[CteFeature]) -> bool {
    matches!(join.kind, JoinKind::Cross)
        && join
            .right_relation
            .as_ref()
            .is_some_and(|rel| ctes.iter().any(|cte| cte.name == *rel))
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
                    evidence::literal("select_star"),
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
        if is_dbt_macro_path(&ctx.file.root_relative_path) {
            return Vec::new();
        }
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
                ) && !join_has_clear_equality(join, &ctx.file.text)
            })
            .map(|join| {
                diagnostic(
                    ctx,
                    self.id(),
                    self.default_severity(),
                    Some(join.span),
                    "Join has no clear equality predicate.",
                    evidence::join_unbounded(join),
                )
                .with_risk(
                    "non-equality joins can cause large intermediate scans and unexpected row multiplication.",
                )
                .with_suggestion("join on stable keys before applying transformations or filters.")
                .with_confidence(if sql.feature_extraction_used_ast {
                    Confidence::Medium
                } else {
                    Confidence::Low
                })
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
                    evidence::literal("order_by"),
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
        if is_dbt_macro_path(&ctx.file.root_relative_path) {
            return Vec::new();
        }
        let Some(sql) = ctx.sql else {
            return Vec::new();
        };
        if !sql.features.group_by_clauses.is_empty()
            || ctx.file.text.to_ascii_lowercase().contains("group by")
        {
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
                    evidence::literal("select_distinct"),
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
            .filter(|join| {
                matches!(join.kind, JoinKind::Cross | JoinKind::Comma)
                    && !is_cte_broadcast_cross_join(join, &sql.features.ctes)
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
                    "Cross join detected without an explicit allow comment.",
                    evidence::join_cross(join),
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
                    evidence::window_text(&window.text),
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
        if is_dbt_macro_path(&ctx.file.root_relative_path) {
            return Vec::new();
        }
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
                    evidence::cte_name(&reference.key),
                )
                .with_risk("some warehouses may recompute expensive CTEs or produce larger plans.")
                .with_suggestion(
                    "materialize the CTE if it is expensive or reused across branches.",
                )]
            })
    }
}

impl Rule for RecursiveCteRule {
    fn id(&self) -> &'static str {
        "SQLCOST044"
    }
    fn name(&self) -> &'static str {
        "Recursive CTE"
    }
    fn description(&self) -> &'static str {
        "Detects WITH RECURSIVE common table expressions."
    }
    fn default_severity(&self) -> Severity {
        Severity::Medium
    }
    fn check(&self, ctx: &RuleContext<'_>) -> Vec<Diagnostic> {
        let Some(sql) = ctx.sql else {
            return Vec::new();
        };
        sql.features
            .recursive_ctes
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
                    "Recursive CTE detected.",
                    evidence::literal("with_recursive"),
                )
                .with_risk(
                    "recursive CTEs can iterate unpredictably and dominate warehouse cost on large graphs.",
                )
                .with_suggestion(
                    "cap recursion depth, pre-materialize graph edges, or use iterative batching when possible.",
                )
                .with_confidence(confidence)
            })
            .collect()
    }
}
