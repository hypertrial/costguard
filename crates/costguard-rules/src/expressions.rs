use crate::evidence;
use crate::helpers::{diagnostic, is_staging_model, threshold};
use crate::registry::{Rule, RuleContext};
use costguard_diagnostics::{Diagnostic, Severity};
use std::collections::HashMap;

pub(crate) struct RepeatedJsonRule;
pub(crate) struct RepeatedRegexRule;
pub(crate) struct RepeatedNormalizationRule;
pub(crate) struct RepeatedExpensiveAcrossFilesRule;

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
                    evidence::expr_key(&feature.key),
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
                    evidence::expr_key(&feature.key),
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
                    evidence::expr_key(&feature.key),
                )
                .with_risk(
                    "repeated canonicalization adds compute and makes logic harder to audit.",
                )
                .with_suggestion("canonicalize once upstream.")]
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
        sql.features
            .json_extractions
            .iter()
            .chain(sql.features.regex_calls.iter())
            .chain(sql.features.normalization_calls.iter())
            .find(|feature| {
                ctx.project_indexes
                    .expensive_expression_file_counts
                    .get(feature.key.as_str())
                    .copied()
                    .unwrap_or(0)
                    >= 3
            })
            .map_or_else(Vec::new, |feature| {
                vec![diagnostic(
                    ctx,
                    self.id(),
                    self.default_severity(),
                    Some(feature.span),
                    "Expensive expression appears in multiple files.",
                    evidence::expr_key(&feature.key),
                )
                .with_risk(
                    "recomputing expensive expressions downstream can compound warehouse cost.",
                )
                .with_suggestion("materialize the expression once upstream.")]
            })
    }
}
