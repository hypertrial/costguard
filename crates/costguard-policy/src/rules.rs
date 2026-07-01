use crate::sign::require_non_empty;
use crate::types::{DeclarativeRule, NumberComparison, PolicyDocumentV2, Predicate, RuleFactsV1};
use anyhow::{Context, Result};
use globset::Glob;
use std::collections::BTreeSet;

pub(crate) const MAX_PREDICATE_DEPTH: usize = 12;
pub(crate) const MAX_REGEX_BYTES: usize = 512;
pub(crate) const MAX_MESSAGE_BYTES: usize = 4096;

pub fn evaluate_custom_rule(rule: &DeclarativeRule, facts: &RuleFactsV1) -> Result<bool> {
    validate_custom_rule(rule)?;
    evaluate_predicate(&rule.predicate, facts)
}

pub fn collect_custom_rule_ids(policy: &PolicyDocumentV2) -> BTreeSet<String> {
    policy
        .scopes
        .iter()
        .flat_map(|scope| scope.custom_rules.iter().map(|rule| rule.id.clone()))
        .collect()
}

pub(crate) fn validate_custom_rule(rule: &DeclarativeRule) -> Result<()> {
    if rule.id.to_ascii_uppercase().starts_with("SQLCOST") || !rule.id.contains('/') {
        anyhow::bail!(
            "custom rule id '{}' must use an organization namespace",
            rule.id
        );
    }
    if rule.message.is_empty() || rule.message.len() > MAX_MESSAGE_BYTES {
        anyhow::bail!(
            "custom rule '{}' message must be 1..={MAX_MESSAGE_BYTES} bytes",
            rule.id
        );
    }
    if rule
        .cost_multiplier
        .is_some_and(|value| !value.is_finite() || value <= 0.0 || value > 1_000.0)
    {
        anyhow::bail!(
            "custom rule '{}' cost_multiplier must be finite and in (0, 1000]",
            rule.id
        );
    }
    validate_predicate(&rule.predicate, 0)
}

fn validate_predicate(predicate: &Predicate, depth: usize) -> Result<()> {
    if depth > MAX_PREDICATE_DEPTH {
        anyhow::bail!("custom rule predicate exceeds maximum depth {MAX_PREDICATE_DEPTH}");
    }
    match predicate {
        Predicate::All { predicates } | Predicate::Any { predicates } => {
            if predicates.is_empty() {
                anyhow::bail!("all/any predicates cannot be empty");
            }
            for child in predicates {
                validate_predicate(child, depth + 1)?;
            }
        }
        Predicate::Not { predicate } => validate_predicate(predicate, depth + 1)?,
        Predicate::Equals { field, .. }
        | Predicate::Glob { field, .. }
        | Predicate::Regex { field, .. } => validate_field(field, FieldKind::String)?,
        Predicate::In { field, .. } => validate_membership_field(field)?,
        Predicate::Number { field, value, .. } => {
            validate_field(field, FieldKind::Number)?;
            if !value.is_finite() {
                anyhow::bail!("numeric predicate value must be finite");
            }
        }
    }
    if let Predicate::Glob { pattern, .. } = predicate {
        Glob::new(pattern).context("invalid declarative-rule glob")?;
    }
    if let Predicate::Regex { pattern, .. } = predicate {
        if pattern.len() > MAX_REGEX_BYTES {
            anyhow::bail!("declarative-rule regex exceeds {MAX_REGEX_BYTES} bytes");
        }
        regex::Regex::new(pattern).context("invalid declarative-rule regex")?;
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum FieldKind {
    String,
    Number,
    Set,
}

fn validate_field(field: &str, expected: FieldKind) -> Result<()> {
    require_non_empty("predicate field", field)?;
    let actual = field_kind(field)
        .with_context(|| format!("unsupported declarative-rule field '{field}'"))?;
    if std::mem::discriminant(&actual) != std::mem::discriminant(&expected) {
        anyhow::bail!("operator is incompatible with declarative-rule field '{field}'");
    }
    Ok(())
}

fn validate_membership_field(field: &str) -> Result<()> {
    require_non_empty("predicate field", field)?;
    match field_kind(field) {
        Some(FieldKind::String | FieldKind::Set) => Ok(()),
        Some(FieldKind::Number) => {
            anyhow::bail!("membership operator is incompatible with numeric field '{field}'")
        }
        None => anyhow::bail!("unsupported declarative-rule field '{field}'"),
    }
}

fn field_kind(field: &str) -> Option<FieldKind> {
    if matches!(
        field,
        "platform"
            | "path"
            | "file.kind"
            | "metadata.state"
            | "dbt.materialized"
            | "dbt.model"
            | "dbt.unique_key"
            | "dbt.incremental_strategy"
            | "dbt.partition_by"
            | "dbt.cluster_by"
            | "dbt.on_schema_change"
            | "sql.parsed"
    ) {
        Some(FieldKind::String)
    } else if matches!(
        field,
        "dbt.tags" | "sql.function_classes" | "sql.join_kinds" | "lineage.refs" | "lineage.sources"
    ) {
        Some(FieldKind::Set)
    } else if matches!(
        field,
        "dbt.ref_count"
            | "dbt.source_count"
            | "sql.join_count"
            | "sql.equality_join_count"
            | "sql.unbounded_join_count"
            | "sql.window_count"
            | "sql.unpartitioned_window_count"
            | "sql.cte_count"
            | "sql.select_star_count"
            | "sql.json_extraction_count"
            | "sql.regex_count"
            | "sql.normalization_count"
            | "sql.non_sargable_count"
            | "sql.correlated_subquery_count"
            | "sql.cross_join_count"
    ) {
        Some(FieldKind::Number)
    } else {
        None
    }
}

fn evaluate_predicate(predicate: &Predicate, facts: &RuleFactsV1) -> Result<bool> {
    Ok(match predicate {
        Predicate::All { predicates } => predicates
            .iter()
            .map(|item| evaluate_predicate(item, facts))
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .all(|matched| matched),
        Predicate::Any { predicates } => predicates
            .iter()
            .map(|item| evaluate_predicate(item, facts))
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .any(|matched| matched),
        Predicate::Not { predicate } => !evaluate_predicate(predicate, facts)?,
        Predicate::Equals { field, value } => facts.strings.get(field) == Some(value),
        Predicate::Number {
            field,
            comparison,
            value,
        } => facts
            .numbers
            .get(field)
            .is_some_and(|actual| match comparison {
                NumberComparison::Eq => actual == value,
                NumberComparison::Gt => actual > value,
                NumberComparison::Gte => actual >= value,
                NumberComparison::Lt => actual < value,
                NumberComparison::Lte => actual <= value,
            }),
        Predicate::Glob { field, pattern } => facts.strings.get(field).is_some_and(|value| {
            Glob::new(pattern)
                .map(|glob| glob.compile_matcher().is_match(value))
                .unwrap_or(false)
        }),
        Predicate::In { field, values } => {
            facts
                .strings
                .get(field)
                .is_some_and(|value| values.contains(value))
                || facts
                    .sets
                    .get(field)
                    .is_some_and(|set| values.iter().any(|value| set.contains(value)))
        }
        Predicate::Regex { field, pattern } => facts.strings.get(field).is_some_and(|value| {
            regex::Regex::new(pattern)
                .map(|regex| regex.is_match(value))
                .unwrap_or(false)
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{DeclarativeRule, NumberComparison, Predicate};
    use costguard_diagnostics::{Confidence, Severity};
    use costguard_protocol::EnforcementMode;

    #[test]
    fn declarative_rules_are_bounded_and_evaluated() {
        let rule = DeclarativeRule {
            id: "acme/no-large-joins".into(),
            severity: Severity::High,
            confidence: Confidence::High,
            enforcement: EnforcementMode::Block,
            message: "too many joins".into(),
            risk: None,
            suggestion: None,
            cost_multiplier: None,
            predicate: Predicate::All {
                predicates: vec![
                    Predicate::Glob {
                        field: "path".into(),
                        pattern: "models/marts/**".into(),
                    },
                    Predicate::Number {
                        field: "sql.join_count".into(),
                        comparison: NumberComparison::Gte,
                        value: 4.0,
                    },
                ],
            },
        };
        let mut facts = RuleFactsV1::default();
        facts
            .strings
            .insert("path".into(), "models/marts/fct.sql".into());
        facts.numbers.insert("sql.join_count".into(), 5.0);
        assert!(evaluate_custom_rule(&rule, &facts).unwrap());
    }
}
