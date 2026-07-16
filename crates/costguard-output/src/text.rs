use costguard_core::ScanResult;
use costguard_cost::format_cost_line;
use costguard_diagnostics::Diagnostic;

use crate::{
    append_analysis_text, append_context_text, append_cost_summary, append_pr_cost_impact_text,
    escape_text, format_diagnostic_meta,
};

pub(crate) fn render_text(result: &ScanResult) -> String {
    let mut output = String::new();
    append_analysis_text(&mut output, result);
    if let Some(summary) = result.cost_summary.as_ref() {
        if let Some(pr) = &summary.pr_impact {
            append_pr_cost_impact_text(&mut output, summary, pr);
        }
    }
    if let Some(summary) = &result.pr_summary {
        if !summary.gate_results.is_empty() {
            output.push_str("PR gates:\n");
            for gate in &summary.gate_results {
                output.push_str(&format!(
                    "  - {}: {}{}\n",
                    gate.name,
                    gate.status,
                    if gate.reasons.is_empty() {
                        String::new()
                    } else {
                        format!(" ({})", gate.reasons.join("; "))
                    }
                ));
            }
            output.push('\n');
        }
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
    append_context_text(&mut output, result.context.as_ref());

    if result.diagnostics.is_empty() {
        output.push_str(&format!(
            "No diagnostics. Scanned {} SQL, {} YAML, {} Python, {} manifest files.\n",
            result.counts.sql, result.counts.yaml, result.counts.python, result.counts.manifest
        ));
        append_cost_summary(&mut output, result.cost_summary.as_ref());
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
        output.push_str(&format!(
            "      {}\n",
            escape_text(&format_diagnostic_meta(diagnostic))
        ));
        if let Some(status) = result.diagnostic_delta_status(diagnostic) {
            let enforcement =
                if result.block_only_new() && !result.diagnostic_is_blocking(diagnostic) {
                    " (nonblocking)"
                } else {
                    ""
                };
            output.push_str(&format!("      PR delta: {status}{enforcement}\n"));
        }
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

pub(crate) fn append_top_cost_findings(output: &mut String, diagnostics: &[Diagnostic]) {
    let mut ranked: Vec<_> = diagnostics
        .iter()
        .filter_map(|d| d.cost_estimate.as_ref().map(|c| (d, c)))
        .collect();
    if ranked.is_empty() {
        return;
    }
    let rank_by_usd = ranked
        .iter()
        .all(|(_, cost)| cost.savings_p50_usd_per_month.is_some());
    ranked.sort_by(|(_left, left_cost), (_right, right_cost)| {
        let left = if rank_by_usd {
            left_cost.savings_p50_usd_per_month.unwrap_or_default()
        } else {
            left_cost.relative_index
        };
        let right = if rank_by_usd {
            right_cost.savings_p50_usd_per_month.unwrap_or_default()
        } else {
            right_cost.relative_index
        };
        right
            .partial_cmp(&left)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    output.push_str("\nTop findings by estimated monthly savings:\n");
    for (diagnostic, cost) in ranked.into_iter().take(5) {
        output.push_str(&format!(
            "  - {} {}:{} — {}\n",
            diagnostic.rule_id,
            diagnostic.path.display(),
            diagnostic.line,
            format_cost_line(cost)
        ));
    }
}

pub(crate) fn render_cost_text(result: &ScanResult) -> String {
    let mut output = String::from("Costguard cost prioritization summary\n\n");
    append_cost_summary(&mut output, result.cost_summary.as_ref());
    if !result.diagnostics.is_empty() {
        append_top_cost_findings(&mut output, &result.diagnostics);
    }
    output
}
