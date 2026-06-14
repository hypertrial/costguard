use crate::evidence;
use crate::helpers::{diagnostic, threshold};
use crate::registry::{Rule, RuleContext};
use costguard_diagnostics::{Confidence, Diagnostic, Severity};
use costguard_sql::JoinKind;

pub(crate) struct FanOutJoinRule;
pub(crate) struct UnmaterializedHeavyViewRule;

fn join_keys_cover_unique_key(join_keys: &[String], unique_key: &[String]) -> bool {
    if unique_key.is_empty() {
        return true;
    }
    unique_key
        .iter()
        .all(|key| join_keys.iter().any(|join_key| join_key == key))
}

fn fan_out_join_diagnostic(
    ctx: &RuleContext<'_>,
    rule_id: &'static str,
    severity: Severity,
    join: &costguard_sql::JoinFeature,
    model_name: &str,
    unique_key: &[String],
) -> Diagnostic {
    diagnostic(
        ctx,
        rule_id,
        severity,
        Some(join.span),
        "Join equality keys do not cover the joined model's known unique_key grain.",
        evidence::join_fan_out(model_name, &join.equality_keys, unique_key),
    )
    .with_risk(
        "many-to-many joins can multiply row counts and create expensive downstream deduplication.",
    )
    .with_suggestion(
        "join on the upstream model's unique_key columns or dedupe the driving side first.",
    )
    .with_confidence(Confidence::Medium)
}

fn fan_out_join_for_model(
    ctx: &RuleContext<'_>,
    join: &costguard_sql::JoinFeature,
    model_name: &str,
    rule_id: &'static str,
    severity: Severity,
) -> Option<Diagnostic> {
    let model_meta = ctx.project_indexes.model_meta.get(model_name)?;
    let unique_key = model_meta.unique_key.as_ref()?;
    if unique_key.is_empty() || join_keys_cover_unique_key(&join.equality_keys, unique_key) {
        return None;
    }
    Some(fan_out_join_diagnostic(
        ctx, rule_id, severity, join, model_name, unique_key,
    ))
}

impl Rule for FanOutJoinRule {
    fn id(&self) -> &'static str {
        "SQLCOST038"
    }
    fn name(&self) -> &'static str {
        "Fan-out join on non-unique key"
    }
    fn description(&self) -> &'static str {
        "Detects equality joins where keys do not cover the joined model's known grain."
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
            .filter(|join| {
                matches!(
                    join.kind,
                    JoinKind::Inner | JoinKind::Left | JoinKind::Right | JoinKind::Full
                ) && join.has_equality
            })
            .filter_map(|join| {
                if let Some(right_relation) = join.right_relation.as_deref() {
                    if let Some(diagnostic) = fan_out_join_for_model(
                        ctx,
                        join,
                        right_relation,
                        self.id(),
                        self.default_severity(),
                    ) {
                        return Some(diagnostic);
                    }
                }
                for dbt_ref in &sql.dbt.refs {
                    if let Some(diagnostic) = fan_out_join_for_model(
                        ctx,
                        join,
                        &dbt_ref.name,
                        self.id(),
                        self.default_severity(),
                    ) {
                        return Some(diagnostic);
                    }
                }
                None
            })
            .collect()
    }
}

impl Rule for UnmaterializedHeavyViewRule {
    fn id(&self) -> &'static str {
        "SQLCOST039"
    }
    fn name(&self) -> &'static str {
        "Heavily referenced view or ephemeral model"
    }
    fn description(&self) -> &'static str {
        "Detects view or ephemeral models referenced by many downstream models."
    }
    fn default_severity(&self) -> Severity {
        Severity::Medium
    }
    fn check(&self, ctx: &RuleContext<'_>) -> Vec<Diagnostic> {
        let Some(sql) = ctx.sql else {
            return Vec::new();
        };
        let materialized = ctx
            .dbt_model
            .and_then(|model| model.materialized.as_deref())
            .or(sql.dbt.config.materialized.as_deref());
        let Some(materialized) = materialized else {
            return Vec::new();
        };
        if materialized != "view" && materialized != "ephemeral" {
            return Vec::new();
        }
        let model_name = ctx
            .file
            .path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or_default();
        let ref_count = ctx
            .project_indexes
            .model_ref_counts
            .get(model_name)
            .copied()
            .unwrap_or(0);
        let min_refs = threshold(ctx, self.id(), 4);
        if ref_count < min_refs {
            return Vec::new();
        }
        vec![diagnostic(
            ctx,
            self.id(),
            self.default_severity(),
            None,
            "View or ephemeral model is referenced by many downstream models.",
            evidence::heavy_view(model_name, materialized),
        )
        .with_risk(
            "heavily referenced views re-execute their full query for every downstream model.",
        )
        .with_suggestion(
            "materialize as table or incremental when the model is reused across the DAG.",
        )
        .with_confidence(Confidence::Medium)]
    }
}
