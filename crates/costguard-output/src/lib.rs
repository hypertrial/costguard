use anyhow::Result;
use costguard_core::{OutputFormat, ScanResult};
use costguard_diagnostics::Diagnostic;
use serde::Serialize;

#[derive(Debug, Serialize)]
struct JsonOutput<'a> {
    diagnostics: &'a [Diagnostic],
}

pub fn render(result: &ScanResult, format: OutputFormat) -> Result<String> {
    match format {
        OutputFormat::Text => Ok(render_text(result)),
        OutputFormat::Json => Ok(serde_json::to_string_pretty(&JsonOutput {
            diagnostics: &result.diagnostics,
        })?),
    }
}

pub fn render_rules(
    rules: &[costguard_core::RuleMetadata],
    format: OutputFormat,
) -> Result<String> {
    match format {
        OutputFormat::Json => Ok(serde_json::to_string_pretty(rules)?),
        OutputFormat::Text => {
            let mut output = String::new();
            for rule in rules {
                output.push_str(&format!(
                    "{}  {:<11} {}\n      {}\n",
                    rule.severity.label(),
                    rule.id,
                    rule.name,
                    rule.description
                ));
            }
            Ok(output)
        }
    }
}

fn render_text(result: &ScanResult) -> String {
    let mut output = String::new();
    if let Some(summary) = &result.pr_summary {
        output.push_str("Changed files:\n");
        if summary.changed_files.is_empty() {
            output.push_str("  none\n");
        } else {
            for path in &summary.changed_files {
                output.push_str(&format!("  - {}\n", path.display()));
            }
        }
        if !summary.changed_models.is_empty() {
            output.push_str("\nChanged dbt models:\n");
            for model in &summary.changed_models {
                output.push_str(&format!("  - {model}\n"));
            }
        }
        if !summary.affected_downstream.is_empty() {
            output.push_str("\nAffected downstream:\n");
            for node in &summary.affected_downstream {
                output.push_str(&format!("  - {node}\n"));
            }
        }
        if let Some(command) = &summary.recommended_dbt_command {
            output.push_str(&format!("\nRecommended dbt command:\n  {command}\n"));
        }
        output.push('\n');
    }

    if result.diagnostics.is_empty() {
        output.push_str(&format!(
            "No diagnostics. Scanned {} SQL, {} YAML, {} Python, {} manifest files.\n",
            result.counts.sql, result.counts.yaml, result.counts.python, result.counts.manifest
        ));
        return output;
    }

    for diagnostic in &result.diagnostics {
        output.push_str(&format!(
            "{}  {} {}:{}:{}\n",
            diagnostic.severity.label(),
            diagnostic.rule_id,
            diagnostic.path.display(),
            diagnostic.line,
            diagnostic.column
        ));
        output.push_str(&format!("      {}\n", diagnostic.message));
        if let Some(risk) = &diagnostic.risk {
            output.push_str(&format!("      Risk: {risk}\n"));
        }
        if let Some(suggestion) = &diagnostic.suggestion {
            output.push_str(&format!("      Suggestion: {suggestion}\n"));
        }
        output.push('\n');
    }
    output
}
