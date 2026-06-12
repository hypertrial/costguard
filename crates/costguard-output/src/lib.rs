use anyhow::Result;
use costguard_core::{OutputFormat, PrSummary, ScanResult};
use costguard_diagnostics::{Diagnostic, Severity};
use serde::Serialize;

#[derive(Debug, Serialize)]
struct JsonOutput<'a> {
    schema_version: u8,
    metrics: &'a costguard_core::ScanMetrics,
    diagnostics: &'a [Diagnostic],
    files: &'a [costguard_core::FileParseStatus],
    #[serde(skip_serializing_if = "Option::is_none")]
    pr_summary: Option<&'a PrSummary>,
}

pub fn render(result: &ScanResult, format: OutputFormat) -> Result<String> {
    match format {
        OutputFormat::Text => Ok(render_text(result)),
        OutputFormat::Json => Ok(serde_json::to_string_pretty(&JsonOutput {
            schema_version: 1,
            metrics: &result.metrics,
            diagnostics: &result.diagnostics,
            files: &result.file_parse_status,
            pr_summary: result.pr_summary.as_ref(),
        })?),
        OutputFormat::Github => Ok(render_github(result)),
        OutputFormat::Markdown => Ok(render_markdown(result)),
        OutputFormat::Sarif => Ok(render_sarif(result)?),
    }
}

pub fn render_rules(
    rules: &[costguard_core::RuleMetadata],
    format: OutputFormat,
) -> Result<String> {
    match format {
        OutputFormat::Json => Ok(serde_json::to_string_pretty(rules)?),
        OutputFormat::Text | OutputFormat::Github => {
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
        OutputFormat::Markdown => {
            let mut output = String::from("| Severity | Rule | Name |\n| --- | --- | --- |\n");
            for rule in rules {
                output.push_str(&format!(
                    "| {} | `{}` | {} |\n",
                    rule.severity.label(),
                    rule.id,
                    escape_markdown(rule.name)
                ));
            }
            Ok(output)
        }
        OutputFormat::Sarif => {
            let payload = serde_json::json!({
                "$schema": "https://json.schemastore.org/sarif-2.1.0.json",
                "version": "2.1.0",
                "runs": [{
                    "tool": {
                        "driver": {
                            "name": "costguard",
                            "version": env!("CARGO_PKG_VERSION"),
                            "informationUri": "https://github.com/hypertrial/costguard",
                            "rules": sarif_rule_definitions(rules)
                        }
                    },
                    "results": []
                }]
            });
            Ok(serde_json::to_string_pretty(&payload)?)
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
        if !summary.affected_exposures.is_empty() {
            output.push_str("\nAffected exposures:\n");
            for exposure in &summary.affected_exposures {
                output.push_str(&format!("  - exposure: {exposure}\n"));
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

fn render_github(result: &ScanResult) -> String {
    let mut output = String::new();
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

fn github_message(diagnostic: &Diagnostic) -> String {
    if let (Some(line), Some(column)) = (diagnostic.compiled_line, diagnostic.compiled_column) {
        return format!(
            "{} (compiled SQL location: line {line}, column {column})",
            diagnostic.message
        );
    }
    diagnostic.message.clone()
}

fn render_markdown(result: &ScanResult) -> String {
    let mut output = String::new();
    let high_count = result
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity >= Severity::High)
        .count();
    if high_count > 0 {
        output.push_str(&format!(
            "# Costguard failed this PR\n\n{high_count} high-risk cost finding"
        ));
        if high_count != 1 {
            output.push('s');
        }
        output.push_str(".\n\n");
    } else {
        output.push_str("# Costguard passed\n\nNo high-risk cost findings.\n\n");
    }

    if let Some(summary) = &result.pr_summary {
        output.push_str("## PR Impact\n\n");
        markdown_list(&mut output, "Changed dbt models", &summary.changed_models);
        markdown_list(
            &mut output,
            "Affected downstream",
            &summary.affected_downstream,
        );
        markdown_list(
            &mut output,
            "Affected exposures",
            &summary.affected_exposures,
        );
        if let Some(command) = &summary.recommended_dbt_command {
            output.push_str(&format!(
                "Recommended dbt command:\n\n```bash\n{command}\n```\n\n"
            ));
        }
    }

    if !result.diagnostics.is_empty() {
        output.push_str("## Diagnostics\n\n");
        for diagnostic in &result.diagnostics {
            output.push_str(&format!(
                "1. `{}` {}:{}:{}\n   {}\n",
                diagnostic.rule_id,
                diagnostic.path.display(),
                diagnostic.line,
                diagnostic.column,
                escape_markdown(&diagnostic.message)
            ));
            if let Some(risk) = &diagnostic.risk {
                output.push_str(&format!("   Risk: {}\n", escape_markdown(risk)));
            }
            if let Some(suggestion) = &diagnostic.suggestion {
                output.push_str(&format!("   Suggestion: {}\n", escape_markdown(suggestion)));
            }
            output.push('\n');
        }
        output.push_str(
            "Suppress only intentional exceptions with `-- costguard: disable-next-line=RULE`.\n",
        );
    }

    output
}

fn markdown_list(output: &mut String, title: &str, items: &[String]) {
    if items.is_empty() {
        return;
    }
    output.push_str(&format!("{title}:\n"));
    for item in items {
        output.push_str(&format!("- {}\n", escape_markdown(item)));
    }
    output.push('\n');
}

fn summary_sentence(summary: &PrSummary) -> String {
    format!(
        "{} changed file(s), {} changed model(s), {} downstream node(s), {} exposure(s)",
        summary.changed_files.len(),
        summary.changed_models.len(),
        summary.affected_downstream.len(),
        summary.affected_exposures.len()
    )
}

fn escape_github_property(value: &str) -> String {
    value
        .replace('%', "%25")
        .replace('\r', "%0D")
        .replace('\n', "%0A")
        .replace(',', "%2C")
        .replace(':', "%3A")
}

fn escape_github_message(value: &str) -> String {
    value
        .replace('%', "%25")
        .replace('\r', "%0D")
        .replace('\n', "%0A")
}

fn escape_markdown(value: &str) -> String {
    value.replace('|', "\\|")
}

fn sarif_level(severity: Severity) -> &'static str {
    match severity {
        Severity::Critical | Severity::High => "error",
        Severity::Medium => "warning",
        Severity::Low | Severity::Info => "note",
    }
}

fn render_sarif(result: &ScanResult) -> Result<String> {
    let rules = costguard_core::rules();
    let payload = serde_json::json!({
        "$schema": "https://json.schemastore.org/sarif-2.1.0.json",
        "version": "2.1.0",
        "runs": [{
            "tool": {
                "driver": {
                    "name": "costguard",
                    "version": env!("CARGO_PKG_VERSION"),
                    "informationUri": "https://github.com/hypertrial/costguard",
                    "rules": sarif_rule_definitions(&rules)
                }
            },
            "results": sarif_results(result)
        }]
    });
    Ok(serde_json::to_string_pretty(&payload)?)
}

fn sarif_results(result: &ScanResult) -> Vec<serde_json::Value> {
    result
        .diagnostics
        .iter()
        .map(|diagnostic| {
            serde_json::json!({
                "ruleId": diagnostic.rule_id,
                "level": sarif_level(diagnostic.severity),
                "message": {
                    "text": diagnostic.message
                },
                "locations": [{
                    "physicalLocation": {
                        "artifactLocation": {
                            "uri": diagnostic.path.to_string_lossy().replace('\\', "/")
                        },
                        "region": {
                            "startLine": diagnostic.line,
                            "startColumn": diagnostic.column
                        }
                    }
                }]
            })
        })
        .collect()
}

fn sarif_rule_definitions(rules: &[costguard_core::RuleMetadata]) -> Vec<serde_json::Value> {
    rules
        .iter()
        .map(|rule| {
            serde_json::json!({
                "id": rule.id,
                "name": rule.name,
                "shortDescription": {
                    "text": rule.description
                },
                "defaultConfiguration": {
                    "level": sarif_level(rule.severity)
                }
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use costguard_core::{ScanMetrics, ScanResult};
    use costguard_diagnostics::{Confidence, Diagnostic, Severity, SourceProvenance};
    use costguard_scanner::ScanCounts;
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    fn sample_result(high: bool) -> ScanResult {
        let diagnostics = if high {
            vec![Diagnostic {
                rule_id: "SQLCOST005".into(),
                severity: Severity::High,
                path: PathBuf::from("models/marts/a,sql"),
                line: 1,
                column: 2,
                span: None,
                message: "line1\nline2".into(),
                risk: Some("risk|note".into()),
                suggestion: None,
                confidence: Confidence::High,
                warehouse: None,
                source_provenance: None,
                compiled_line: None,
                compiled_column: None,
            }]
        } else {
            Vec::new()
        };
        ScanResult {
            diagnostics,
            counts: ScanCounts::default(),
            metrics: ScanMetrics {
                counts: ScanCounts::default(),
                sql_parse_total: 0,
                sql_parse_failures: 0,
                sql_parse_other_total: 0,
                sql_parse_other_failures: 0,
                sql_parse_compiled_total: 0,
                sql_parse_compiled_failures: 0,
                metadata_warnings: 0,
                yaml_parse_failures: 0,
                dbt_project_parse_failures: 0,
                metadata_only_scan: false,
                diagnostics_by_rule: BTreeMap::new(),
                diagnostics_by_severity: BTreeMap::new(),
                baselined_findings: 0,
                new_findings: 0,
            },
            file_parse_status: Vec::new(),
            pr_summary: Some(PrSummary {
                changed_models: vec!["a".into()],
                affected_downstream: vec!["b".into()],
                affected_exposures: vec!["dashboard".into()],
                ..PrSummary::default()
            }),
        }
    }

    #[test]
    fn escape_github_property_escapes_commas_and_colons() {
        let rendered = render_github(&sample_result(true));
        assert!(rendered.contains("file=models/marts/a%2Csql"));
        assert!(rendered.contains("title=SQLCOST005"));
        assert!(rendered.contains("::error file="));
    }

    #[test]
    fn escape_github_message_preserves_newlines_as_pct() {
        let rendered = render_github(&sample_result(true));
        assert!(rendered.contains("line1%0Aline2"));
    }

    #[test]
    fn render_markdown_failed_header_counts_high_only() {
        let rendered = render_markdown(&sample_result(true));
        assert!(rendered.contains("# Costguard failed this PR"));
        assert!(rendered.contains("1 high-risk cost finding."));
    }

    #[test]
    fn render_markdown_includes_pr_impact_sections() {
        let rendered = render_markdown(&sample_result(true));
        assert!(rendered.contains("Changed dbt models"));
        assert!(rendered.contains("Affected downstream"));
        assert!(rendered.contains("Affected exposures"));
        assert!(rendered.contains("- dashboard"));
    }

    #[test]
    fn escape_markdown_pipes() {
        let rendered = render_markdown(&sample_result(true));
        assert!(rendered.contains("risk\\|note"));
    }

    #[test]
    fn render_markdown_pass_header_when_no_high_findings() {
        let rendered = render_markdown(&sample_result(false));
        assert!(rendered.contains("# Costguard passed"));
    }

    #[test]
    fn json_output_includes_metrics() {
        let mut result = sample_result(true);
        result.metrics.sql_parse_total = 3;
        result.metrics.sql_parse_failures = 1;
        result
            .metrics
            .diagnostics_by_rule
            .insert("SQLCOST005".into(), 1);
        result
            .metrics
            .diagnostics_by_severity
            .insert("high".into(), 1);

        let rendered = render(&result, OutputFormat::Json).expect("json render");
        let value: serde_json::Value = serde_json::from_str(&rendered).expect("parse json");
        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["metrics"]["sql_parse_total"], 3);
        assert_eq!(value["metrics"]["sql_parse_failures"], 1);
        assert_eq!(value["metrics"]["diagnostics_by_rule"]["SQLCOST005"], 1);
        assert_eq!(value["diagnostics"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn json_output_includes_compiled_provenance() {
        let mut result = sample_result(true);
        result.diagnostics[0].source_provenance = Some(SourceProvenance::CompiledUnmapped);
        result.diagnostics[0].compiled_line = Some(12);
        result.diagnostics[0].compiled_column = Some(4);

        let rendered = render(&result, OutputFormat::Json).expect("json render");
        let value: serde_json::Value = serde_json::from_str(&rendered).expect("parse json");
        assert_eq!(
            value["diagnostics"][0]["source_provenance"],
            "compiled_unmapped"
        );
        assert_eq!(value["diagnostics"][0]["compiled_line"], 12);
        assert_eq!(value["diagnostics"][0]["compiled_column"], 4);
    }

    #[test]
    fn github_output_mentions_compiled_location() {
        let mut result = sample_result(true);
        result.diagnostics[0].source_provenance = Some(SourceProvenance::CompiledUnmapped);
        result.diagnostics[0].compiled_line = Some(12);
        result.diagnostics[0].compiled_column = Some(4);
        let rendered = render_github(&result);
        assert!(rendered.contains("compiled SQL location: line 12"));
    }

    #[test]
    fn sarif_output_has_required_fields() {
        let rendered = render(&sample_result(true), OutputFormat::Sarif).expect("sarif render");
        let value: serde_json::Value = serde_json::from_str(&rendered).expect("parse sarif");
        assert_eq!(value["version"], "2.1.0");
        assert_eq!(value["runs"][0]["tool"]["driver"]["name"], "costguard");
        assert_eq!(value["runs"][0]["results"][0]["ruleId"], "SQLCOST005");
    }
}
