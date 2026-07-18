//! Scan result rendering.
//!
//! Formats [`ScanResult`] as text, JSON (schema v4),
//! GitHub workflow annotations, markdown, or SARIF.

use anyhow::Result;
use costguard_core::{OutputFormat, PrSummary, ReceiptTrend, ScanResult};
use costguard_cost::{format_usd_interval, CostFigure, PrCostImpact, ProjectCostSummary};
use costguard_diagnostics::{Confidence, Diagnostic};
use serde::{Deserialize, Serialize};

const OUTPUT_SCHEMA_VERSION: u8 = 4;

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

#[derive(Debug, Deserialize)]
struct ReceiptV4 {
    schema_version: u8,
    diagnostics: Vec<ReceiptDiagnostic>,
    cost: Option<ReceiptCost>,
}

#[derive(Debug, Deserialize)]
struct ReceiptDiagnostic {
    severity: String,
}

#[derive(Debug, Deserialize)]
struct ReceiptCost {
    project_p50_usd: Option<f64>,
    savings_p50_usd: Option<f64>,
    savings_gb_months: Option<f64>,
    coverage: Option<ReceiptCoverage>,
}

#[derive(Debug, Deserialize)]
struct ReceiptCoverage {
    models_total: Option<usize>,
    models_with_usd: Option<usize>,
    usd_coverage_fraction: Option<f64>,
}

/// Compare a JSON schema-v4 receipt with a current scan result.
pub fn receipt_trend(previous_json: &str, current: &ScanResult) -> Result<ReceiptTrend> {
    let previous: ReceiptV4 =
        serde_json::from_str(previous_json).map_err(|error| anyhow::anyhow!(error))?;
    if previous.schema_version != OUTPUT_SCHEMA_VERSION {
        anyhow::bail!(
            "receipt must use JSON schema version {}",
            OUTPUT_SCHEMA_VERSION
        );
    }
    let previous_high = previous
        .diagnostics
        .iter()
        .filter(|diagnostic| matches!(diagnostic.severity.as_str(), "high" | "critical"))
        .count();
    let current_high = current
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity >= costguard_diagnostics::Severity::High)
        .count();
    let previous_cost = previous.cost.as_ref();
    let previous_usd_comparable = previous_cost.is_some_and(receipt_has_complete_usd);
    let current_usd_comparable = current.cost_summary.as_ref().is_some_and(|cost| {
        cost.coverage.models_total > 0
            && cost.coverage.models_with_usd == cost.coverage.models_total
    });
    let previous_usd = previous_usd_comparable
        .then(|| previous_cost.and_then(|cost| cost.savings_p50_usd))
        .flatten();
    let current_usd = current_usd_comparable
        .then(|| {
            current
                .cost_summary
                .as_ref()
                .and_then(|cost| cost.savings_p50_usd)
        })
        .flatten();
    let previous_gb = previous_cost.and_then(|cost| cost.savings_gb_months);
    let current_gb = current
        .cost_summary
        .as_ref()
        .map(|cost| cost.savings_gb_months);
    Ok(ReceiptTrend {
        diagnostics_delta: current.diagnostics.len() as i64 - previous.diagnostics.len() as i64,
        high_findings_delta: current_high as i64 - previous_high as i64,
        monthly_savings_delta_usd: previous_usd
            .zip(current_usd)
            .map(|(previous, current)| current - previous),
        monthly_savings_delta_gb: previous_gb
            .zip(current_gb)
            .map(|(previous, current)| current - previous),
    })
}

fn receipt_has_complete_usd(cost: &ReceiptCost) -> bool {
    let coverage = cost.coverage.as_ref();
    match coverage.map(|value| (value.models_total, value.models_with_usd)) {
        Some((Some(total), Some(with_usd))) => total > 0 && with_usd == total,
        Some((Some(_), None) | (None, Some(_))) => false,
        _ => coverage
            .and_then(|value| value.usd_coverage_fraction)
            .map(|fraction| fraction == 1.0)
            .unwrap_or(cost.project_p50_usd.is_some()),
    }
}

pub(crate) fn ranked_cost_findings<'a>(
    diagnostics: &'a [Diagnostic],
    summary: Option<&ProjectCostSummary>,
) -> Vec<(&'a Diagnostic, &'a costguard_diagnostics::CostEstimate)> {
    let mut ranked = diagnostics
        .iter()
        .filter_map(|diagnostic| {
            diagnostic
                .cost_estimate
                .as_ref()
                .map(|cost| (diagnostic, cost))
        })
        .collect::<Vec<_>>();
    let rank_by_usd = summary.is_some_and(has_full_usd_coverage);
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
    ranked
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

const COST_DISCLAIMER: &str = "Note: advisory priors from local data (grades A/B/C), not a bill. Severity/confidence enforce gates. See cost-estimates docs.";

fn append_cost_grade_caveat(output: &mut String, summary: &ProjectCostSummary, prefix: &str) {
    if summary.grade_a == 0 && summary.coverage.models_total > 0 {
        output.push_str(&format!(
            "{prefix}All estimates use size priors only (grade C); no measured spend mapped.\n"
        ));
    }
}

fn append_cost_disclaimer(output: &mut String, prefix: &str) {
    output.push_str(&format!("{prefix}{COST_DISCLAIMER}\n"));
}

pub(crate) fn format_diagnostic_meta(diagnostic: &Diagnostic) -> String {
    let mut parts = vec![
        format!("severity {}", diagnostic.severity.label()),
        format!("confidence {}", confidence_label(diagnostic.confidence)),
    ];
    if let Some(tier) = &diagnostic.rule_precision_tier {
        parts.push(format!("precision {tier}"));
    }
    if !diagnostic.governance.owners.is_empty() {
        parts.push(format!("owners {}", diagnostic.governance.owners.join("/")));
    }
    parts.join(", ")
}

fn confidence_label(confidence: Confidence) -> &'static str {
    match confidence {
        Confidence::Low => "low",
        Confidence::Medium => "medium",
        Confidence::High => "high",
    }
}

pub(crate) fn append_pr_cost_impact_text(
    output: &mut String,
    summary: &ProjectCostSummary,
    pr: &PrCostImpact,
) {
    output.push_str("PR Cost Impact:\n");
    append_cost_figure_line(
        output,
        &covered_cost_label(summary, "  Net", &pr.net),
        &pr.net,
    );
    append_cost_figure_line(
        output,
        &covered_cost_label(summary, "  Introduced", &pr.introduced),
        &pr.introduced,
    );
    append_cost_figure_line(
        output,
        &covered_cost_label(summary, "  Avoided", &pr.avoided),
        &pr.avoided,
    );
    append_cost_figure_line(
        output,
        &covered_cost_label(summary, "  Efficiency", &pr.efficiency),
        &pr.efficiency,
    );
    append_cost_figure_line(
        output,
        &covered_cost_label(summary, "  Volume", &pr.volume),
        &pr.volume,
    );
    append_cost_figure_line(
        output,
        &covered_cost_label(summary, "  Blast radius (downstream)", &pr.blast_radius),
        &pr.blast_radius,
    );
    output.push_str(&format!(
        "  Coverage: {:.0}% USD ({}/{} models); {:.0}% mapped spend\n",
        summary.coverage.usd_coverage_fraction * 100.0,
        summary.coverage.models_with_usd,
        summary.coverage.models_total,
        summary.coverage.mapped_spend_fraction * 100.0,
    ));
    if let (Some(p10), Some(p50), Some(p90)) = (
        summary.savings_p10_usd,
        summary.savings_p50_usd,
        summary.savings_p90_usd,
    ) {
        output.push_str(&format!(
            "  Addressable savings on flagged findings{}: ~${:.0}/mo (${:.0}–${:.0})\n",
            mapped_usd_suffix(summary),
            p50,
            p10,
            p90
        ));
    }
    if summary.savings_gb_months > 0.0 {
        output.push_str(&format!(
            "  Addressable savings on flagged findings: ~{:.0} GB-mo\n",
            summary.savings_gb_months
        ));
    }
    output.push_str(&format!(
        "  Grade mix: A={}, B={}, C={}\n",
        summary.grade_a, summary.grade_b, summary.grade_c
    ));
    if !summary.top_models.is_empty() {
        output.push_str(if has_full_usd_coverage(summary) {
            "  Top models by USD cost:\n"
        } else {
            "  Top models by volume:\n"
        });
        for model in summary.top_models.iter().take(5) {
            if let Some(p50) = model.p50_usd_per_month {
                output.push_str(&format!(
                    "    - {} — ~${p50:.0}/mo; {:.0} GB-mo (grade {}, {})\n",
                    escape_text(&model.model_id),
                    model.gb_months,
                    model.grade,
                    model.estimation_basis
                ));
            } else {
                output.push_str(&format!(
                    "    - {} — {:.0} GB-mo (grade {}, {})\n",
                    escape_text(&model.model_id),
                    model.gb_months,
                    model.grade,
                    model.estimation_basis
                ));
            }
        }
    }
    append_cost_grade_caveat(output, summary, "  ");
    append_cost_disclaimer(output, "  ");
    output.push('\n');
}

pub(crate) fn append_pr_cost_impact_markdown(
    output: &mut String,
    summary: &ProjectCostSummary,
    pr: &PrCostImpact,
) {
    output.push_str("## PR Cost Impact\n\n");
    append_cost_figure_markdown(
        output,
        &covered_cost_label(summary, "Net", &pr.net),
        &pr.net,
    );
    append_cost_figure_markdown(
        output,
        &covered_cost_label(summary, "Introduced", &pr.introduced),
        &pr.introduced,
    );
    append_cost_figure_markdown(
        output,
        &covered_cost_label(summary, "Avoided", &pr.avoided),
        &pr.avoided,
    );
    append_cost_figure_markdown(
        output,
        &covered_cost_label(summary, "Efficiency", &pr.efficiency),
        &pr.efficiency,
    );
    append_cost_figure_markdown(
        output,
        &covered_cost_label(summary, "Volume", &pr.volume),
        &pr.volume,
    );
    append_cost_figure_markdown(
        output,
        &covered_cost_label(summary, "Blast radius (downstream)", &pr.blast_radius),
        &pr.blast_radius,
    );
    output.push_str(&format!(
        "- **Coverage:** {:.0}% USD ({}/{} models); {:.0}% mapped spend\n",
        summary.coverage.usd_coverage_fraction * 100.0,
        summary.coverage.models_with_usd,
        summary.coverage.models_total,
        summary.coverage.mapped_spend_fraction * 100.0,
    ));
    if let (Some(p10), Some(p50), Some(p90)) = (
        summary.savings_p10_usd,
        summary.savings_p50_usd,
        summary.savings_p90_usd,
    ) {
        output.push_str(&format!(
            "- **Addressable savings{}:** ~${:.0}/mo (${:.0}–${:.0})\n",
            mapped_usd_suffix(summary),
            p50,
            p10,
            p90
        ));
    }
    if summary.savings_gb_months > 0.0 {
        output.push_str(&format!(
            "- **Addressable savings:** ~{:.0} GB-mo\n",
            summary.savings_gb_months
        ));
    }
    output.push_str(&format!(
        "- **Grade mix:** A={}, B={}, C={}\n\n",
        summary.grade_a, summary.grade_b, summary.grade_c
    ));
    if !summary.top_models.is_empty() {
        output.push_str(if has_full_usd_coverage(summary) {
            "### Top models by USD cost\n\n"
        } else {
            "### Top models by volume\n\n"
        });
        for model in summary.top_models.iter().take(5) {
            if let Some(p50) = model.p50_usd_per_month {
                output.push_str(&format!(
                    "- `{}` — ~${p50:.0}/mo; {:.0} GB-mo (grade {})\n",
                    model.model_id, model.gb_months, model.grade
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
    append_cost_grade_caveat(output, summary, "");
    append_cost_disclaimer(output, "");
    output.push('\n');
}

pub(crate) fn append_cost_summary(output: &mut String, summary: Option<&ProjectCostSummary>) {
    let Some(summary) = summary else {
        return;
    };
    output.push_str("\nCost summary:\n");
    append_cost_figure_line(
        output,
        &covered_cost_label(summary, "  Current cost", &summary.current_cost),
        &summary.current_cost,
    );
    append_cost_figure_line(
        output,
        &covered_cost_label(summary, "  Post-fix cost", &summary.post_fix_cost),
        &summary.post_fix_cost,
    );
    append_cost_figure_line(
        output,
        &covered_cost_label(
            summary,
            "  Potential savings (current - post-fix)",
            &summary.potential_savings,
        ),
        &summary.potential_savings,
    );
    if let Some(pr) = &summary.pr_impact {
        append_cost_figure_line(output, "  PR impact (net)", &pr.net);
        append_cost_figure_line(output, "    efficiency", &pr.efficiency);
        append_cost_figure_line(output, "    volume", &pr.volume);
        append_cost_figure_line(output, "    blast radius (downstream)", &pr.blast_radius);
    }
    if let Some(realized) = &summary.realized_savings {
        let label = if realized.realized.monthly_p50.is_some() {
            "  Realized savings (mapped USD)"
        } else {
            "  Realized savings"
        };
        append_cost_figure_line(output, label, &realized.realized);
    }
    output.push_str(&format!(
        "  Coverage: {:.0}% USD ({}/{} models); {:.0}% mapped spend",
        summary.coverage.usd_coverage_fraction * 100.0,
        summary.coverage.models_with_usd,
        summary.coverage.models_total,
        summary.coverage.mapped_spend_fraction * 100.0,
    ));
    if let Some(age) = summary.coverage.observation_age_days {
        output.push_str(&format!(", observation age {:.0}d", age));
    }
    output.push('\n');
    if let (Some(p10), Some(p50), Some(p90)) = (
        summary.savings_p10_usd,
        summary.savings_p50_usd,
        summary.savings_p90_usd,
    ) {
        output.push_str(&format!(
            "  Addressable savings on flagged findings (deduplicated){}: ~${:.0}/mo (${:.0}–${:.0})\n",
            mapped_usd_suffix(summary),
            p50, p10, p90
        ));
    }
    if summary.savings_gb_months > 0.0 {
        output.push_str(&format!(
            "  Addressable savings on flagged findings (deduplicated): ~{:.0} GB-mo\n",
            summary.savings_gb_months
        ));
    }
    output.push_str(&format!(
        "  Grade mix: A={}, B={}, C={}\n",
        summary.grade_a, summary.grade_b, summary.grade_c
    ));
    if !summary.top_models.is_empty() {
        output.push_str(if has_full_usd_coverage(summary) {
            "  Top models by USD cost:\n"
        } else {
            "  Top models by volume:\n"
        });
        for model in &summary.top_models {
            if let Some(p50) = model.p50_usd_per_month {
                output.push_str(&format!(
                    "    - {} — ~${p50:.0}/mo; {:.0} GB-mo (grade {}, {})\n",
                    escape_markdown(&model.model_id),
                    model.gb_months,
                    model.grade,
                    model.estimation_basis
                ));
            } else {
                output.push_str(&format!(
                    "    - {} — {:.0} GB-mo (grade {}, {})\n",
                    escape_markdown(&model.model_id),
                    model.gb_months,
                    model.grade,
                    model.estimation_basis
                ));
            }
        }
    }
    append_cost_grade_caveat(output, summary, "  ");
    append_cost_disclaimer(output, "  ");
}

fn append_cost_figure_line(output: &mut String, label: &str, figure: &CostFigure) {
    let volume = figure
        .gb_months_p50
        .map(|value| format!("; ~{value:.0} GB-mo"))
        .unwrap_or_default();
    if let (Some(p10), Some(p50), Some(p90)) =
        (figure.monthly_p10, figure.monthly_p50, figure.monthly_p90)
    {
        let annual = figure
            .annual_p50
            .map(|value| format!(" / ~${value:.0}/yr"))
            .unwrap_or_default();
        output.push_str(&format!(
            "{label}: {}{}{volume} ({})\n",
            format_usd_interval(p10, p50, p90),
            annual,
            figure.basis
        ));
    } else if let Some(p50) = figure.monthly_p50 {
        output.push_str(&format!(
            "{label}: ~${p50:.0}/mo{volume} ({})\n",
            figure.basis
        ));
    } else if let Some(gb) = figure.gb_months_p50 {
        output.push_str(&format!("{label}: ~{gb:.0} GB-mo ({})\n", figure.basis));
    }
}

fn append_cost_figure_markdown(output: &mut String, label: &str, figure: &CostFigure) {
    let volume = figure
        .gb_months_p50
        .map(|value| format!(", ~{value:.0} GB-mo"))
        .unwrap_or_default();
    if let Some(p50) = figure.monthly_p50 {
        let annual = figure
            .annual_p50
            .map(|value| format!(", ~${value:.0}/yr"))
            .unwrap_or_default();
        output.push_str(&format!(
            "- **{label}:** ~${p50:.0}/mo{annual}{volume} ({})\n",
            figure.basis
        ));
    } else if let Some(gb) = figure.gb_months_p50 {
        output.push_str(&format!(
            "- **{label}:** ~{gb:.0} GB-mo ({})\n",
            figure.basis
        ));
    }
}

fn mapped_usd_suffix(summary: &ProjectCostSummary) -> &'static str {
    if summary.coverage.models_with_usd > 0 && !has_full_usd_coverage(summary) {
        " (mapped USD)"
    } else {
        ""
    }
}

fn covered_cost_label(summary: &ProjectCostSummary, label: &str, figure: &CostFigure) -> String {
    if figure.monthly_p50.is_some() {
        format!("{label}{}", mapped_usd_suffix(summary))
    } else {
        label.into()
    }
}

pub(crate) fn has_full_usd_coverage(summary: &ProjectCostSummary) -> bool {
    summary.coverage.models_total > 0
        && summary.coverage.models_with_usd == summary.coverage.models_total
}

pub(crate) fn append_cost_summary_markdown(
    output: &mut String,
    summary: Option<&ProjectCostSummary>,
) {
    let Some(summary) = summary else {
        return;
    };
    output.push_str("\n## Cost summary\n\n");
    append_cost_figure_markdown(
        output,
        &covered_cost_label(summary, "Current cost", &summary.current_cost),
        &summary.current_cost,
    );
    append_cost_figure_markdown(
        output,
        &covered_cost_label(summary, "Post-fix cost", &summary.post_fix_cost),
        &summary.post_fix_cost,
    );
    append_cost_figure_markdown(
        output,
        &covered_cost_label(
            summary,
            "Potential savings (current - post-fix)",
            &summary.potential_savings,
        ),
        &summary.potential_savings,
    );
    if let Some(pr) = &summary.pr_impact {
        append_cost_figure_markdown(output, "PR impact (net)", &pr.net);
        append_cost_figure_markdown(output, "Blast radius (downstream)", &pr.blast_radius);
    }
    if let Some(realized) = &summary.realized_savings {
        let label = if realized.realized.monthly_p50.is_some() {
            "Realized savings (mapped USD)"
        } else {
            "Realized savings"
        };
        append_cost_figure_markdown(output, label, &realized.realized);
    }
    output.push_str(&format!(
        "Coverage: {:.0}% USD ({}/{} models); {:.0}% mapped spend.\n\n",
        summary.coverage.usd_coverage_fraction * 100.0,
        summary.coverage.models_with_usd,
        summary.coverage.models_total,
        summary.coverage.mapped_spend_fraction * 100.0,
    ));
    if let Some(p50) = summary.savings_p50_usd {
        output.push_str(&format!(
            "Addressable savings on flagged findings (deduplicated){}: ~${:.0}/mo.\n\n",
            mapped_usd_suffix(summary),
            p50
        ));
    }
    if summary.savings_gb_months > 0.0 {
        output.push_str(&format!(
            "Addressable savings on flagged findings (deduplicated): ~{:.0} GB-mo.\n\n",
            summary.savings_gb_months
        ));
    }
    output.push_str(&format!(
        "Input grades: A={}, B={}, C={}.\n\n",
        summary.grade_a, summary.grade_b, summary.grade_c
    ));
    if !summary.top_models.is_empty() {
        output.push_str(if has_full_usd_coverage(summary) {
            "### Top models by USD cost\n\n"
        } else {
            "### Top models by volume\n\n"
        });
        for model in &summary.top_models {
            if let Some(p50) = model.p50_usd_per_month {
                output.push_str(&format!(
                    "- `{}` — ~${p50:.0}/mo; {:.0} GB-mo (grade {})\n",
                    model.model_id, model.gb_months, model.grade
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
    append_cost_grade_caveat(output, summary, "");
    append_cost_disclaimer(output, "");
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
    use costguard_core::{
        EnforcementPreview, FindingDelta, GateMode, GateResult, GateStatus, ScanMetrics, ScanResult,
    };
    use costguard_diagnostics::{
        Confidence, CostEstimate, CostGrade, Diagnostic, Severity, SourceProvenance,
    };
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
                rule_precision_tier: None,
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

    fn unchanged_regression_only_result() -> ScanResult {
        let mut result = sample_result(true);
        result.diagnostics[0].assign_identity("sem-v1:unchanged");
        let finding_id = result.diagnostics[0].governance.finding_id.clone();
        let summary = result.pr_summary.as_mut().expect("PR summary");
        summary.finding_delta = Some(FindingDelta {
            unchanged: 1,
            ..FindingDelta::default()
        });
        summary.enforcement_preview = Some(EnforcementPreview {
            block_only_new: true,
            require_manifest_integrity: false,
            require_rocky_artifact_integrity: false,
        });
        assert!(!finding_id.is_empty());
        result
    }

    fn cost_diagnostic(path: &str, relative_index: f64, usd: f64) -> Diagnostic {
        let mut diagnostic = Diagnostic::new(
            "SQLCOST001",
            Severity::Medium,
            PathBuf::from(path),
            None,
            "test",
        );
        diagnostic.cost_estimate = Some(CostEstimate {
            relative_index,
            grade: CostGrade::B,
            basis: "test".into(),
            prior_basis: None,
            currency: "USD".into(),
            model_id: None,
            model_monthly_p50_usd: None,
            savings_p10_usd_per_month: Some(usd),
            savings_p50_usd_per_month: Some(usd),
            savings_p90_usd_per_month: Some(usd),
            current_cost_p50_usd_per_month: None,
            post_fix_cost_p50_usd_per_month: None,
            unestimated_reason: None,
            downstream_model_count: None,
            downstream_monthly_p50_usd: None,
        });
        diagnostic
    }

    #[test]
    fn finding_ranking_uses_usd_only_for_complete_project_coverage() {
        let diagnostics = vec![
            cost_diagnostic("high-volume.sql", 100.0, 1.0),
            cost_diagnostic("high-usd.sql", 10.0, 10.0),
        ];
        let mut summary = ProjectCostSummary {
            coverage: costguard_cost::CoverageMetrics {
                models_total: 2,
                models_with_usd: 1,
                usd_coverage_fraction: 0.5,
                ..Default::default()
            },
            ..ProjectCostSummary::default()
        };

        assert_eq!(
            ranked_cost_findings(&diagnostics, Some(&summary))[0].0.path,
            PathBuf::from("high-volume.sql")
        );

        summary.coverage.models_with_usd = 2;
        summary.coverage.usd_coverage_fraction = 1.0;
        assert_eq!(
            ranked_cost_findings(&diagnostics, Some(&summary))[0].0.path,
            PathBuf::from("high-usd.sql")
        );
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
    fn github_output_reports_gate_failures_and_downgrades_exceptions() {
        let mut result = sample_result(true);
        result.diagnostics[0].governance.enforcement =
            costguard_protocol::EnforcementOutcome::Excepted;
        result.pr_summary.as_mut().unwrap().gate_results = vec![GateResult {
            name: "finance".into(),
            mode: GateMode::Block,
            status: GateStatus::Fail,
            matched_findings: 1,
            reasons: vec!["cost threshold exceeded".into()],
        }];
        let rendered = render_github(&result);
        assert!(rendered.contains("::notice file="), "{rendered}");
        assert!(
            rendered.contains("::error title=Costguard gate finance::cost threshold exceeded"),
            "{rendered}"
        );
    }

    #[test]
    fn render_markdown_failed_header_counts_high_only() {
        let rendered = render_markdown(&sample_result(true));
        assert!(rendered.contains("# Costguard failed this PR"));
        assert!(rendered.contains("1 high-risk cost finding."));
    }

    #[test]
    fn regression_only_output_keeps_unchanged_finding_nonblocking() {
        let result = unchanged_regression_only_result();
        let markdown = render_markdown(&result);
        assert!(markdown.contains("# Costguard passed"), "{markdown}");
        assert!(
            markdown.contains("No introduced or regressed high-risk cost findings"),
            "{markdown}"
        );
        assert!(
            markdown.contains("PR delta: unchanged (nonblocking)"),
            "{markdown}"
        );
        assert!(markdown.contains("Active enforcement:"), "{markdown}");

        let github = render_github(&result);
        assert!(github.contains("::notice file="), "{github}");
        assert!(
            github.contains("PR delta: unchanged (nonblocking)"),
            "{github}"
        );
    }

    #[test]
    fn regression_only_missing_identity_remains_blocking() {
        let mut result = unchanged_regression_only_result();
        result.diagnostics[0].governance.finding_id.clear();
        let markdown = render_markdown(&result);
        assert!(
            markdown.contains("# Costguard failed this PR"),
            "{markdown}"
        );
        assert!(markdown.contains("PR delta: unknown"), "{markdown}");
    }

    #[test]
    fn github_pr_summary_includes_finding_delta_and_priced_net_impact() {
        let mut result = unchanged_regression_only_result();
        result.cost_summary = Some(ProjectCostSummary {
            project_p50_usd: Some(1_000.0),
            pr_impact: Some(PrCostImpact {
                net: CostFigure {
                    monthly_p50: Some(125.0),
                    ..CostFigure::default()
                },
                ..PrCostImpact::default()
            }),
            ..ProjectCostSummary::default()
        });
        let github = render_github(&result);
        assert!(github.contains("finding delta: 0 introduced"), "{github}");
        assert!(github.contains("net PR impact: $125/mo"), "{github}");
    }

    #[test]
    fn render_markdown_includes_pr_impact_sections() {
        let rendered = render_markdown(&sample_result(true));
        assert!(rendered.contains("Changed models"));
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
        assert_eq!(value["schema_version"], 4);
        assert_eq!(value["analysis"]["policy"], "standard");
        assert_eq!(value["analysis"]["passed"], true);
        assert!(value["cost"].is_null());
        assert_eq!(value["metrics"]["sql_parse_total"], 3);
        assert_eq!(value["metrics"]["sql_parse_failures"], 1);
        assert_eq!(value["metrics"]["diagnostics_by_rule"]["SQLCOST005"], 1);
        assert_eq!(value["diagnostics"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn receipt_trend_uses_typed_v4_coverage_rules() {
        let mut current = sample_result(false);
        current.cost_summary = Some(ProjectCostSummary {
            project_p50_usd: Some(120.0),
            savings_p50_usd: Some(30.0),
            savings_gb_months: 4.0,
            coverage: costguard_cost::CoverageMetrics {
                models_total: 2,
                models_with_usd: 2,
                usd_coverage_fraction: 1.0,
                ..Default::default()
            },
            ..ProjectCostSummary::default()
        });
        let previous = r#"{
            "schema_version": 4,
            "diagnostics": [{"severity": "high"}],
            "cost": {
                "project_p50_usd": 100.0,
                "savings_p50_usd": 20.0,
                "savings_gb_months": 1.5,
                "coverage": {
                    "models_total": 2,
                    "models_with_usd": 2,
                    "usd_coverage_fraction": 1.0
                }
            },
            "unknown_future_field": true
        }"#;
        let trend = receipt_trend(previous, &current).expect("trend");
        assert_eq!(trend.high_findings_delta, -1);
        assert_eq!(trend.monthly_savings_delta_usd, Some(10.0));
        assert_eq!(trend.monthly_savings_delta_gb, Some(2.5));
    }

    #[test]
    fn receipt_trend_suppresses_partial_usd_and_supports_legacy_v4() {
        let mut current = sample_result(false);
        current.cost_summary = Some(ProjectCostSummary {
            project_p50_usd: Some(120.0),
            savings_p50_usd: Some(30.0),
            coverage: costguard_cost::CoverageMetrics {
                models_total: 2,
                models_with_usd: 2,
                usd_coverage_fraction: 1.0,
                ..Default::default()
            },
            ..ProjectCostSummary::default()
        });
        let partial = r#"{
            "schema_version": 4,
            "diagnostics": [],
            "cost": {
                "project_p50_usd": 100.0,
                "savings_p50_usd": 20.0,
                "coverage": {"usd_coverage_fraction": 0.5}
            }
        }"#;
        assert_eq!(
            receipt_trend(partial, &current)
                .expect("partial trend")
                .monthly_savings_delta_usd,
            None
        );

        let legacy = r#"{
            "schema_version": 4,
            "diagnostics": [],
            "cost": {"project_p50_usd": 100.0, "savings_p50_usd": 20.0}
        }"#;
        assert_eq!(
            receipt_trend(legacy, &current)
                .expect("legacy trend")
                .monthly_savings_delta_usd,
            Some(10.0)
        );
    }

    #[test]
    fn receipt_trend_rejects_wrong_schema_and_missing_diagnostics() {
        let current = sample_result(false);
        assert!(receipt_trend(r#"{"schema_version":3,"diagnostics":[]}"#, &current).is_err());
        assert!(receipt_trend(r#"{"schema_version":4}"#, &current).is_err());
    }

    #[test]
    fn unpriced_cost_summary_renders_volume_without_dollars() {
        let mut result = sample_result(false);
        result.diagnostics.clear();
        result.pr_summary = None;
        result.cost_summary = Some(ProjectCostSummary {
            project_gb_months: 125.0,
            current_cost: CostFigure {
                gb_months_p50: Some(125.0),
                basis: "project-total".into(),
                ..CostFigure::default()
            },
            coverage: costguard_cost::CoverageMetrics {
                models_total: 2,
                ..Default::default()
            },
            model_count: 2,
            ..ProjectCostSummary::default()
        });
        let markdown = render_cost_report(&result, OutputFormat::Markdown).unwrap();
        assert!(markdown.contains("125 GB-mo"), "{markdown}");
        assert!(!markdown.contains('$'), "{markdown}");

        for format in [OutputFormat::Text, OutputFormat::Markdown] {
            let rendered = render(&result, format).unwrap();
            assert!(rendered.contains("125 GB-mo"), "{rendered}");
            assert!(!rendered.contains('$'), "{rendered}");
        }

        let json = render(&result, OutputFormat::Json).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(value["cost"]["current_cost"]["monthly_p50"].is_null());
        assert_eq!(value["cost"]["current_cost"]["gb_months_p50"], 125.0);
        assert!(value["cost"]["savings_p50_usd"].is_null());
    }

    #[test]
    fn partial_coverage_labels_only_figures_that_contain_usd() {
        let mut result = sample_result(false);
        result.pr_summary = None;
        result.cost_summary = Some(ProjectCostSummary {
            current_cost: CostFigure {
                monthly_p50: Some(100.0),
                gb_months_p50: Some(1_000.0),
                basis: "mapped-project-total".into(),
                ..CostFigure::default()
            },
            potential_savings: CostFigure {
                gb_months_p50: Some(25.0),
                basis: "potential-savings".into(),
                ..CostFigure::default()
            },
            coverage: costguard_cost::CoverageMetrics {
                models_total: 1_000,
                models_with_usd: 999,
                usd_coverage_fraction: 0.999,
                ..Default::default()
            },
            ..ProjectCostSummary::default()
        });

        let markdown = render_cost_report(&result, OutputFormat::Markdown).unwrap();

        assert!(
            markdown.contains("**Current cost (mapped USD):**"),
            "{markdown}"
        );
        assert!(
            markdown.contains("**Potential savings (current - post-fix):** ~25 GB-mo"),
            "{markdown}"
        );
        assert!(!markdown.contains("Potential savings (current - post-fix) (mapped USD)"));
        assert!(markdown.contains("Top models by volume") || !markdown.contains("Top models"));
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
