//! Scan result rendering.
//!
//! Formats [`ScanResult`] as text, JSON (schema v3),
//! GitHub workflow annotations, markdown, or SARIF.

use anyhow::Result;
use costguard_core::{OutputFormat, PrSummary, ScanResult};
use costguard_cost::{format_usd_interval, ProjectCostSummary};
use costguard_diagnostics::Diagnostic;
use serde::Serialize;

const OUTPUT_SCHEMA_VERSION: u8 = 3;

#[derive(Debug, Serialize)]
struct JsonOutput<'a> {
    schema_version: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    identity_scheme: Option<&'a str>,
    run: &'a costguard_core::RunMetadata,
    policy: &'a costguard_core::PolicyMetadata,
    analysis: &'a costguard_core::AnalysisReport,
    metrics: &'a costguard_core::ScanMetrics,
    #[serde(skip_serializing_if = "Option::is_none")]
    cost: Option<&'a ProjectCostSummary>,
    diagnostics: &'a [Diagnostic],
    files: &'a [costguard_core::FileParseStatus],
    #[serde(skip_serializing_if = "Option::is_none")]
    pr_summary: Option<&'a PrSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    context: Option<&'a costguard_core::ContextReport>,
}

/// Render a scan result in the requested [`OutputFormat`].
pub fn render(result: &ScanResult, format: OutputFormat) -> Result<String> {
    match format {
        OutputFormat::Text => Ok(render_text(result)),
        OutputFormat::Json => Ok(serde_json::to_string_pretty(&JsonOutput {
            schema_version: OUTPUT_SCHEMA_VERSION,
            identity_scheme: result.identity_scheme.as_deref(),
            run: &result.run,
            policy: &result.policy,
            analysis: &result.analysis,
            metrics: &result.metrics,
            cost: result.cost_summary.as_ref(),
            diagnostics: &result.diagnostics,
            files: &result.file_parse_status,
            pr_summary: result.pr_summary.as_ref(),
            context: result.context.as_ref(),
        })?),
        OutputFormat::Github => Ok(render_github(result)),
        OutputFormat::Markdown => Ok(render_markdown(result)),
        OutputFormat::Sarif => Ok(render_sarif(result)?),
    }
}

pub fn render_cost_report(result: &ScanResult, format: OutputFormat) -> Result<String> {
    match format {
        OutputFormat::Json => Ok(serde_json::to_string_pretty(&result.cost_summary)?),
        OutputFormat::Markdown => Ok(render_cost_markdown(result)),
        _ => Ok(render_cost_text(result)),
    }
}

/// Render rule metadata in the requested output format.
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

mod github;
mod markdown;
mod sarif;
mod text;

use github::render_github;
use markdown::{render_cost_markdown, render_markdown};
use sarif::{render_sarif, sarif_rule_definitions};
use text::{render_cost_text, render_text};

pub(crate) fn append_cost_summary(output: &mut String, summary: Option<&ProjectCostSummary>) {
    let Some(summary) = summary else {
        return;
    };
    output.push_str("\nCost summary:\n");
    if let (Some(p10), Some(p50), Some(p90)) = (
        summary.project_p10_usd,
        summary.project_p50_usd,
        summary.project_p90_usd,
    ) {
        output.push_str(&format!(
            "  Project total: {} across {} models\n",
            format_usd_interval(p10, p50, p90),
            summary.model_count
        ));
    } else {
        output.push_str(&format!(
            "  Project scan volume: {:.0} GB-mo across {} models\n",
            summary.project_gb_months, summary.model_count
        ));
    }
    if summary.savings_p50_usd > 0.0 {
        output.push_str(&format!(
            "  New finding savings (deduplicated): ~${:.0}/mo (${:.0}–${:.0})\n",
            summary.savings_p50_usd, summary.savings_p10_usd, summary.savings_p90_usd
        ));
    } else if summary.savings_gb_months > 0.0 {
        output.push_str(&format!(
            "  New finding savings (deduplicated): ~{:.0} GB-mo\n",
            summary.savings_gb_months
        ));
    }
    output.push_str(&format!(
        "  Grade mix: A={}, B={}, C={}\n",
        summary.grade_a, summary.grade_b, summary.grade_c
    ));
    if !summary.top_models.is_empty() {
        output.push_str("  Top models by cost:\n");
        for model in &summary.top_models {
            if let Some(p50) = model.p50_usd_per_month {
                output.push_str(&format!(
                    "    - {} — ~${p50:.0}/mo (grade {})\n",
                    escape_markdown(&model.model_id),
                    model.grade
                ));
            } else {
                output.push_str(&format!(
                    "    - {} — {:.0} GB-mo (grade {})\n",
                    escape_markdown(&model.model_id),
                    model.gb_months,
                    model.grade
                ));
            }
        }
    }
}

pub(crate) fn append_cost_summary_markdown(
    output: &mut String,
    summary: Option<&ProjectCostSummary>,
) {
    let Some(summary) = summary else {
        return;
    };
    output.push_str("\n## Cost summary\n\n");
    if let (Some(p10), Some(p50), Some(p90)) = (
        summary.project_p10_usd,
        summary.project_p50_usd,
        summary.project_p90_usd,
    ) {
        output.push_str(&format!(
            "Project total: {} across {} models.\n\n",
            format_usd_interval(p10, p50, p90),
            summary.model_count
        ));
    } else {
        output.push_str(&format!(
            "Project scan volume: {:.0} GB-mo across {} models.\n\n",
            summary.project_gb_months, summary.model_count
        ));
    }
    if summary.savings_p50_usd > 0.0 {
        output.push_str(&format!(
            "New finding savings (deduplicated): ~${:.0}/mo.\n\n",
            summary.savings_p50_usd
        ));
    } else if summary.savings_gb_months > 0.0 {
        output.push_str(&format!(
            "New finding savings (deduplicated): ~{:.0} GB-mo.\n\n",
            summary.savings_gb_months
        ));
    }
    output.push_str(&format!(
        "Input grades: A={}, B={}, C={}.\n\n",
        summary.grade_a, summary.grade_b, summary.grade_c
    ));
    if !summary.top_models.is_empty() {
        output.push_str("### Top models by cost\n\n");
        for model in &summary.top_models {
            if let Some(p50) = model.p50_usd_per_month {
                output.push_str(&format!(
                    "- `{}` — ~${p50:.0}/mo (grade {})\n",
                    model.model_id, model.grade
                ));
            } else {
                output.push_str(&format!(
                    "- `{}` — {:.0} GB-mo (grade {})\n",
                    model.model_id, model.gb_months, model.grade
                ));
            }
        }
        output.push('\n');
    }
}

pub(crate) fn escape_github_property(value: &str) -> String {
    escape_github(value, true)
}

pub(crate) fn escape_github_message(value: &str) -> String {
    escape_github(value, false)
}

pub(crate) fn escape_markdown(value: &str) -> String {
    let mut escaped = String::new();
    for ch in value.chars() {
        match ch {
            '\\' => escaped.push_str("\\\\"),
            '`' => escaped.push_str("\\`"),
            '|' => escaped.push_str("\\|"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            ch if ch.is_control() => escaped.push_str(&format!("\\u{:04X}", ch as u32)),
            ch => escaped.push(ch),
        }
    }
    escaped
}

pub(crate) fn escape_fenced_code(value: &str) -> String {
    escape_text(value).replace("```", "` ` `")
}

pub(crate) fn escape_text(value: &str) -> String {
    let mut escaped = String::new();
    for ch in value.chars() {
        match ch {
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            ch if ch.is_control() => escaped.push_str(&format!("\\u{:04X}", ch as u32)),
            ch => escaped.push(ch),
        }
    }
    escaped
}

fn escape_github(value: &str, property: bool) -> String {
    let mut escaped = String::new();
    for ch in value.chars() {
        match ch {
            '%' => escaped.push_str("%25"),
            ',' if property => escaped.push_str("%2C"),
            ':' if property => escaped.push_str("%3A"),
            ch if ch.is_ascii_control() => {
                escaped.push_str(&format!("%{:02X}", ch as u32));
            }
            ch => escaped.push(ch),
        }
    }
    escaped
}

pub(crate) fn append_context_text(
    output: &mut String,
    context: Option<&costguard_core::ContextReport>,
) {
    let Some(context) = context else {
        return;
    };
    output.push_str("Context (informational):\n");
    output.push_str(&format!(
        "  {} SQL files, {} parse failures, {} skipped\n",
        context.counts.sql, context.sql_parse_failures, context.skipped_count
    ));
    if !context.issues.is_empty() {
        output.push_str(&format!(
            "  {} unchanged context issues\n",
            context.issues.len()
        ));
        for issue in context.issues.iter().take(5) {
            output.push_str(&format!(
                "    - {} {}: {}\n",
                issue.rule_id, issue.path, issue.message
            ));
        }
        if context.issues.len() > 5 {
            output.push_str(&format!("    ... and {} more\n", context.issues.len() - 5));
        }
    }
    output.push('\n');
}

pub(crate) fn append_analysis_text(output: &mut String, result: &ScanResult) {
    if result.analysis.passed {
        return;
    }
    output.push_str("Analysis incomplete:\n");
    for violation in &result.analysis.violations {
        output.push_str(&format!(
            "  - {}: {}\n",
            escape_text(&violation.code),
            escape_text(&violation.message)
        ));
    }
    output.push('\n');
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
                governance: Default::default(),
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
                cost_estimate: None,
            }]
        } else {
            Vec::new()
        };
        ScanResult {
            run: costguard_core::RunMetadata {
                id: "test-run".into(),
                started_at: "2026-01-01T00:00:00Z".into(),
                completed_at: "2026-01-01T00:00:01Z".into(),
                duration_ms: 1,
                tool_version: env!("CARGO_PKG_VERSION").into(),
            },
            policy: costguard_core::PolicyMetadata {
                digest: "local-unmanaged".into(),
                version: env!("CARGO_PKG_VERSION").into(),
                scope: "local".into(),
            },
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
            cost_summary: None,
            analysis: costguard_core::AnalysisReport::default(),
            context: None,
            identity_scheme: None,
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
        assert_eq!(value["schema_version"], 3);
        assert_eq!(value["analysis"]["policy"], "standard");
        assert_eq!(value["analysis"]["passed"], true);
        assert!(value["cost"].is_null());
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
        let mut result = sample_result(true);
        result.diagnostics[0].assign_identity("raw:select-star");
        let rendered = render(&result, OutputFormat::Sarif).expect("sarif render");
        let value: serde_json::Value = serde_json::from_str(&rendered).expect("parse sarif");
        assert_eq!(value["version"], "2.1.0");
        assert_eq!(value["runs"][0]["tool"]["driver"]["name"], "costguard");
        assert_eq!(value["runs"][0]["results"][0]["ruleId"], "SQLCOST005");
        assert_eq!(
            value["runs"][0]["properties"]["costguard"]["policy"]["digest"],
            "local-unmanaged"
        );
        let properties = &value["runs"][0]["results"][0]["properties"];
        assert!(properties["findingId"]
            .as_str()
            .is_some_and(|value| !value.is_empty()));
        assert_eq!(properties["evidenceKey"], "raw:select-star");
        assert_eq!(properties["confidence"], "high");
        assert_eq!(properties["enforcementOutcome"], "observed");
        assert!(properties.get("policyProvenance").is_some());
        assert!(properties.get("appliedException").is_some());
    }

    #[test]
    fn markdown_and_workflow_output_escape_control_characters() {
        let mut result = sample_result(true);
        result.diagnostics[0].path = PathBuf::from("models/`bad|name\n.sql");
        let markdown = render_markdown(&result);
        assert!(markdown.contains("\\`bad\\|name\\n.sql"), "{markdown}");
        let github = render_github(&result);
        assert!(github.contains("%0A"), "{github}");
        assert_eq!(github.lines().count(), 2, "{github}");
    }
}
