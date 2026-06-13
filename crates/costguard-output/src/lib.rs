use anyhow::Result;
use costguard_core::{OutputFormat, PrSummary, ScanResult};
use costguard_cost::{format_cost_line, format_usd_interval, ProjectCostSummary};
use costguard_diagnostics::{Diagnostic, Severity};
use serde::Serialize;

#[derive(Debug, Serialize)]
struct JsonOutput<'a> {
    schema_version: u8,
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
}

pub fn render(result: &ScanResult, format: OutputFormat) -> Result<String> {
    match format {
        OutputFormat::Text => Ok(render_text(result)),
        OutputFormat::Json => Ok(serde_json::to_string_pretty(&JsonOutput {
            schema_version: costguard_protocol::SCAN_SCHEMA_VERSION,
            run: &result.run,
            policy: &result.policy,
            analysis: &result.analysis,
            metrics: &result.metrics,
            cost: result.cost_summary.as_ref(),
            diagnostics: &result.diagnostics,
            files: &result.file_parse_status,
            pr_summary: result.pr_summary.as_ref(),
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
    append_analysis_text(&mut output, result);
    if let Some(summary) = &result.pr_summary {
        output.push_str("Changed files:\n");
        if summary.changed_files.is_empty() {
            output.push_str("  none\n");
        } else {
            for path in &summary.changed_files {
                output.push_str(&format!(
                    "  - {}\n",
                    escape_text(&path.display().to_string())
                ));
            }
        }
        if !summary.changed_models.is_empty() {
            output.push_str("\nChanged dbt models:\n");
            for model in &summary.changed_models {
                output.push_str(&format!("  - {}\n", escape_text(model)));
            }
        }
        if !summary.affected_downstream.is_empty() {
            output.push_str("\nAffected downstream:\n");
            for node in &summary.affected_downstream {
                output.push_str(&format!("  - {}\n", escape_text(node)));
            }
        }
        if !summary.affected_exposures.is_empty() {
            output.push_str("\nAffected exposures:\n");
            for exposure in &summary.affected_exposures {
                output.push_str(&format!("  - exposure: {}\n", escape_text(exposure)));
            }
        }
        if let Some(command) = &summary.recommended_dbt_command {
            output.push_str(&format!(
                "\nRecommended dbt command:\n  {}\n",
                escape_text(command)
            ));
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
            escape_text(&diagnostic.path.display().to_string()),
            diagnostic.line,
            diagnostic.column
        ));
        output.push_str(&format!("      {}\n", escape_text(&diagnostic.message)));
        if let Some(risk) = &diagnostic.risk {
            output.push_str(&format!("      Risk: {risk}\n"));
        }
        if let Some(suggestion) = &diagnostic.suggestion {
            output.push_str(&format!("      Suggestion: {suggestion}\n"));
        }
        if let Some(cost) = &diagnostic.cost_estimate {
            output.push_str(&format!("      {}\n", format_cost_line(cost)));
        }
        output.push('\n');
    }
    append_top_cost_findings(&mut output, &result.diagnostics);
    append_cost_summary(&mut output, result.cost_summary.as_ref());
    output
}

fn append_top_cost_findings(output: &mut String, diagnostics: &[Diagnostic]) {
    let mut ranked: Vec<_> = diagnostics
        .iter()
        .filter_map(|d| d.cost_estimate.as_ref().map(|c| (d, c)))
        .collect();
    if ranked.is_empty() {
        return;
    }
    ranked.sort_by(|(_left, left_cost), (_right, right_cost)| {
        right_cost
            .savings_p50_usd_per_month
            .unwrap_or(right_cost.relative_index)
            .partial_cmp(
                &left_cost
                    .savings_p50_usd_per_month
                    .unwrap_or(left_cost.relative_index),
            )
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    output.push_str("\nTop findings by estimated monthly savings:\n");
    for (diagnostic, cost) in ranked.into_iter().take(10) {
        output.push_str(&format!(
            "  - {} {}:{} — {}\n",
            diagnostic.rule_id,
            diagnostic.path.display(),
            diagnostic.line,
            format_cost_line(cost)
        ));
    }
}

fn render_github(result: &ScanResult) -> String {
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

fn github_message(diagnostic: &Diagnostic) -> String {
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

fn render_markdown(result: &ScanResult) -> String {
    let mut output = String::new();
    let high_count = result
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity >= Severity::High)
        .count();
    if !result.analysis.passed {
        output.push_str("# Costguard analysis incomplete\n\n");
        for violation in &result.analysis.violations {
            output.push_str(&format!(
                "- `{}` {}\n",
                escape_markdown(&violation.code),
                escape_markdown(&violation.message)
            ));
        }
        output.push('\n');
    } else if high_count > 0 {
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
                "Recommended dbt command:\n\n```bash\n{}\n```\n\n",
                escape_fenced_code(command)
            ));
        }
    }

    if !result.diagnostics.is_empty() {
        output.push_str("## Diagnostics\n\n");
        for diagnostic in &result.diagnostics {
            output.push_str(&format!(
                "1. `{}` {}:{}:{}\n   {}\n",
                diagnostic.rule_id,
                escape_markdown(&diagnostic.path.display().to_string()),
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
            if let Some(cost) = &diagnostic.cost_estimate {
                output.push_str(&format!(
                    "   {}\n",
                    escape_markdown(&format_cost_line(cost))
                ));
            }
            output.push('\n');
        }
        append_top_cost_findings_markdown(&mut output, &result.diagnostics);
        append_cost_summary_markdown(&mut output, result.cost_summary.as_ref());
        output.push_str(
            "Suppress only intentional exceptions with `-- costguard: disable-next-line=RULE`.\n",
        );
    }

    output
}

fn append_top_cost_findings_markdown(output: &mut String, diagnostics: &[Diagnostic]) {
    let mut ranked: Vec<_> = diagnostics
        .iter()
        .filter_map(|d| d.cost_estimate.as_ref().map(|c| (d, c)))
        .collect();
    if ranked.is_empty() {
        return;
    }
    ranked.sort_by(|(_left, left_cost), (_right, right_cost)| {
        right_cost
            .savings_p50_usd_per_month
            .unwrap_or(right_cost.relative_index)
            .partial_cmp(
                &left_cost
                    .savings_p50_usd_per_month
                    .unwrap_or(left_cost.relative_index),
            )
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    output.push_str("\n## Top findings by estimated monthly savings\n\n");
    for (diagnostic, cost) in ranked.into_iter().take(10) {
        output.push_str(&format!(
            "- `{}` {}:{} — {}\n",
            diagnostic.rule_id,
            escape_markdown(&diagnostic.path.display().to_string()),
            diagnostic.line,
            escape_markdown(&format_cost_line(cost))
        ));
    }
    output.push('\n');
}

fn append_cost_summary(output: &mut String, summary: Option<&ProjectCostSummary>) {
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

fn append_cost_summary_markdown(output: &mut String, summary: Option<&ProjectCostSummary>) {
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

fn render_cost_text(result: &ScanResult) -> String {
    let mut output = String::from("Costguard project cost report\n\n");
    append_cost_summary(&mut output, result.cost_summary.as_ref());
    if !result.diagnostics.is_empty() {
        append_top_cost_findings(&mut output, &result.diagnostics);
    }
    output
}

fn render_cost_markdown(result: &ScanResult) -> String {
    let mut output = String::from("# Costguard project cost report\n\n");
    append_cost_summary_markdown(&mut output, result.cost_summary.as_ref());
    if !result.diagnostics.is_empty() {
        append_top_cost_findings_markdown(&mut output, &result.diagnostics);
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
    escape_github(value, true)
}

fn escape_github_message(value: &str) -> String {
    escape_github(value, false)
}

fn escape_markdown(value: &str) -> String {
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

fn escape_fenced_code(value: &str) -> String {
    escape_text(value).replace("```", "` ` `")
}

fn escape_text(value: &str) -> String {
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

fn append_analysis_text(output: &mut String, result: &ScanResult) {
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
            let mut result = serde_json::json!({
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
            });
            if let Some(cost) = &diagnostic.cost_estimate {
                result["properties"] = serde_json::json!({
                    "costEstimate": cost
                });
            }
            result
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
        let rendered = render(&sample_result(true), OutputFormat::Sarif).expect("sarif render");
        let value: serde_json::Value = serde_json::from_str(&rendered).expect("parse sarif");
        assert_eq!(value["version"], "2.1.0");
        assert_eq!(value["runs"][0]["tool"]["driver"]["name"], "costguard");
        assert_eq!(value["runs"][0]["results"][0]["ruleId"], "SQLCOST005");
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
