use crate::helpers::diagnostic;
use crate::registry::{Rule, RuleContext};
use costguard_diagnostics::{Diagnostic, Severity};
use regex::Regex;
use std::sync::OnceLock;

pub(crate) struct PythonRowWiseRule;

impl Rule for PythonRowWiseRule {
    fn id(&self) -> &'static str {
        "SQLCOST010"
    }
    fn name(&self) -> &'static str {
        "Python model row-wise operation"
    }
    fn description(&self) -> &'static str {
        "Detects row-wise pandas patterns in Python dbt models."
    }
    fn default_severity(&self) -> Severity {
        Severity::High
    }
    fn check(&self, ctx: &RuleContext<'_>) -> Vec<Diagnostic> {
        if ctx.file.kind != costguard_scanner::FileKind::Python {
            return Vec::new();
        }
        row_wise_regex()
            .find_iter(&ctx.file.text)
            .map(|matched| {
                diagnostic(
                    ctx,
                    self.id(),
                    self.default_severity(),
                    Some(ctx.file.line_index.span(matched.start(), matched.end())),
                    "Python model uses row-wise dataframe logic.",
                )
                .with_risk(
                    "row-wise Python logic is often much slower and more memory-intensive than vectorized logic.",
                )
                .with_suggestion("vectorize, use SQL, or use dataframe-native column operations.")
            })
            .collect()
    }
}

fn row_wise_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r#"(?i)\.apply\s*\([^\n]*axis\s*=\s*1|\.iterrows\s*\(|\.itertuples\s*\("#)
            .expect("valid python regex")
    })
}
