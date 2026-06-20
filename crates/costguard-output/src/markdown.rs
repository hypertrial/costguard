use costguard_core::{GateStatus, PrSummary, ScanResult};
use costguard_cost::format_cost_line;
use costguard_diagnostics::{Diagnostic, Severity};

use crate::{
    append_cost_summary_markdown, append_pr_cost_impact_markdown, escape_fenced_code,
    escape_markdown, format_diagnostic_meta,
};

pub(crate) fn render_markdown(result: &ScanResult) -> String {
    let mut output = String::new();
    let high_count = result
        .diagnostics
        .iter()
        .filter(|diagnostic| {
            diagnostic.severity >= Severity::High
                && diagnostic.governance.enforcement
                    != costguard_protocol::EnforcementOutcome::Excepted
                && (result.policy.digest == "local-unmanaged"
                    || diagnostic.governance.enforcement
                        == costguard_protocol::EnforcementOutcome::Blocked)
        })
        .count();
    let blocking_gate = result.pr_summary.as_ref().is_some_and(|summary| {
        summary
            .gate_results
            .iter()
            .any(|gate| gate.status == GateStatus::Fail)
    });
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
    } else if blocking_gate || high_count > 0 {
        output.push_str("# Costguard failed this PR\n\n");
        if high_count > 0 {
            output.push_str(&format!("{high_count} high-risk cost finding"));
            if high_count != 1 {
                output.push('s');
            }
            output.push_str(".\n\n");
        }
        if blocking_gate {
            output.push_str("A blocking PR gate failed.\n\n");
        }
    } else {
        output.push_str("# Costguard passed\n\nNo high-risk cost findings.\n\n");
    }

    if let Some(summary) = result.cost_summary.as_ref() {
        if let Some(pr) = &summary.pr_impact {
            append_pr_cost_impact_markdown(&mut output, summary, pr);
        }
    }

    if let Some(summary) = &result.pr_summary {
        if !summary.gate_results.is_empty() {
            output.push_str("## Gates\n\n");
            for gate in &summary.gate_results {
                output.push_str(&format!(
                    "- **{}:** {}",
                    escape_markdown(&gate.name),
                    match gate.status {
                        GateStatus::Pass => "pass",
                        GateStatus::Warn => "warn",
                        GateStatus::Fail => "fail",
                    }
                ));
                if !gate.reasons.is_empty() {
                    output.push_str(&format!(" — {}", escape_markdown(&gate.reasons.join("; "))));
                }
                output.push('\n');
            }
            output.push('\n');
        }
        output.push_str("## PR Impact\n\n");
        markdown_list(&mut output, "Changed dbt models", &summary.changed_models);
        markdown_list(
            &mut output,
            "Affected downstream",
            &summary.affected_downstream,
        );
        if !summary.owner_summary.is_empty() {
            output.push_str("Changed model owners:\n");
            for (owner, count) in &summary.owner_summary {
                output.push_str(&format!(
                    "- {}: {} model(s)\n",
                    escape_markdown(owner),
                    count
                ));
            }
            output.push('\n');
        }
        if let Some(trend) = &summary.trend {
            output.push_str(&format!(
                "Receipt trend: {:+} findings, {:+} high findings",
                trend.diagnostics_delta, trend.high_findings_delta
            ));
            if let Some(delta) = trend.monthly_savings_delta_usd {
                output.push_str(&format!(", {delta:+.0} USD/mo"));
            } else if let Some(delta) = trend.monthly_savings_delta_gb {
                output.push_str(&format!(", {delta:+.0} GB-mo"));
            }
            output.push_str(".\n\n");
        }
        if let Some(delta) = &summary.finding_delta {
            output.push_str(&format!(
                "Finding delta vs base: {} introduced, {} regressed, {} resolved, {} unchanged.\n\n",
                delta.introduced, delta.regressed, delta.resolved, delta.unchanged
            ));
        }
        if !summary.indirectly_affected.is_empty() {
            output.push_str("Indirect impact:\n");
            for impact in &summary.indirectly_affected {
                output.push_str(&format!(
                    "- {} — {}\n",
                    escape_markdown(&impact.model),
                    escape_markdown(&impact.reason)
                ));
            }
            output.push('\n');
        }
        if let Some(integrity) = &summary.manifest_integrity {
            output.push_str(&format!(
                "Manifest integrity: {} ({} checksum mismatch(es)).\n\n",
                if integrity.verified {
                    "verified"
                } else {
                    "cannot verify head"
                },
                integrity.checksum_mismatches
            ));
        }
        if let Some(preview) = &summary.enforcement_preview {
            if preview.block_only_new || preview.require_manifest_integrity {
                output.push_str(&format!(
                    "Enforcement preview (advisory): block_only_new={}, require_manifest_integrity={}.\n\n",
                    preview.block_only_new, preview.require_manifest_integrity
                ));
            }
        }
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

    if let Some(context) = &result.context {
        output.push_str("## Context (informational)\n\n");
        output.push_str(&format!(
            "- {} SQL files in project context\n",
            context.counts.sql
        ));
        output.push_str(&format!(
            "- {} parse failures, {} skipped files (unchanged)\n",
            context.sql_parse_failures, context.skipped_count
        ));
        if !context.issues.is_empty() {
            output.push_str(&format!(
                "- {} unchanged context issues (nonblocking)\n",
                context.issues.len()
            ));
        }
        output.push('\n');
    }

    if !result.diagnostics.is_empty() {
        output.push_str("## Diagnostics\n\n");
        for diagnostic in &result.diagnostics {
            output.push_str(&format!(
                "1. `{}` {}:{}:{}\n   {}\n   {}\n",
                diagnostic.rule_id,
                escape_markdown(&diagnostic.path.display().to_string()),
                diagnostic.line,
                diagnostic.column,
                escape_markdown(&diagnostic.message),
                escape_markdown(&format_diagnostic_meta(diagnostic))
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

pub(crate) fn append_top_cost_findings_markdown(output: &mut String, diagnostics: &[Diagnostic]) {
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
    for (diagnostic, cost) in ranked.into_iter().take(5) {
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

pub(crate) fn render_cost_markdown(result: &ScanResult) -> String {
    let mut output = String::from("# Costguard cost prioritization summary\n\n");
    append_cost_summary_markdown(&mut output, result.cost_summary.as_ref());
    if !result.diagnostics.is_empty() {
        append_top_cost_findings_markdown(&mut output, &result.diagnostics);
    }
    output
}

pub(crate) fn markdown_list(output: &mut String, title: &str, items: &[String]) {
    if items.is_empty() {
        return;
    }
    output.push_str(&format!("{title}:\n"));
    for item in items {
        output.push_str(&format!("- {}\n", escape_markdown(item)));
    }
    output.push('\n');
}

pub(crate) fn summary_sentence(summary: &PrSummary) -> String {
    format!(
        "{} changed file(s), {} changed model(s), {} downstream node(s), {} exposure(s)",
        summary.changed_files.len(),
        summary.changed_models.len(),
        summary.affected_downstream.len(),
        summary.affected_exposures.len()
    )
}
