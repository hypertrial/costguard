use costguard_core::ScanResult;
use costguard_cost::format_cost_line;
use costguard_diagnostics::{Diagnostic, Severity};

use crate::markdown::summary_sentence;
use crate::{escape_github_message, escape_github_property};

pub(crate) fn render_github(result: &ScanResult) -> String {
    let mut output = String::new();
    for violation in &result.analysis.violations {
        output.push_str(&format!(
            "::error title=Costguard analysis {}::{}\n",
            escape_github_property(&violation.code),
            escape_github_message(&violation.message)
        ));
    }
    for diagnostic in &result.diagnostics {
        let level = match diagnostic.severity {
            Severity::Critical | Severity::High => "error",
            Severity::Medium => "warning",
            Severity::Low | Severity::Info => "notice",
        };
        output.push_str(&format!(
            "::{level} file={},line={},col={},title={} {}::{}\n",
            escape_github_property(&diagnostic.path.display().to_string()),
            diagnostic.line,
            diagnostic.column,
            escape_github_property(&diagnostic.rule_id),
            escape_github_property(diagnostic.severity.label()),
            escape_github_message(&github_message(diagnostic))
        ));
    }
    if let Some(summary) = &result.pr_summary {
        output.push_str(&format!(
            "::notice title=Costguard PR Summary::{}\n",
            escape_github_message(&summary_sentence(summary))
        ));
    }
    output
}

pub(crate) fn github_message(diagnostic: &Diagnostic) -> String {
    let mut message = if let (Some(line), Some(column)) =
        (diagnostic.compiled_line, diagnostic.compiled_column)
    {
        format!(
            "{} (compiled SQL location: line {line}, column {column})",
            diagnostic.message
        )
    } else {
        diagnostic.message.clone()
    };
    if let Some(cost) = &diagnostic.cost_estimate {
        message.push_str(&format!(" | {}", format_cost_line(cost)));
    }
    message
}
