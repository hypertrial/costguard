use crate::config::ScanConfig;
use crate::context::ContextReport;
use crate::{AnalysisReport, AnalysisViolation, ScanMetrics};
use costguard_cost::ProjectCostSummary;

pub(crate) struct AnalysisViolationInput<'a> {
    pub config: &'a ScanConfig,
    pub metrics: &'a ScanMetrics,
    pub target_skipped_files: usize,
    pub metadata_only: bool,
    pub manifest_stale: bool,
    pub pr_mode: bool,
    pub context: Option<&'a ContextReport>,
    pub policy_violations: Vec<costguard_policy::PolicyViolation>,
    pub waiver_violations: Vec<AnalysisViolation>,
    pub cost_summary: Option<&'a ProjectCostSummary>,
}

pub(crate) fn collect_analysis_violations(input: AnalysisViolationInput<'_>) -> AnalysisReport {
    let AnalysisViolationInput {
        config,
        metrics,
        target_skipped_files,
        metadata_only,
        manifest_stale,
        pr_mode,
        context,
        policy_violations,
        waiver_violations,
        cost_summary,
    } = input;
    let mut analysis = evaluate_analysis(
        config,
        metrics,
        target_skipped_files,
        metadata_only,
        manifest_stale,
        pr_mode,
        context,
    );
    for violation in policy_violations {
        analysis.violations.push(AnalysisViolation {
            code: violation.code,
            message: violation.message,
            observed: 1.0,
            allowed: 0.0,
        });
    }
    analysis.violations.extend(waiver_violations);
    if let Some(violation) = evaluate_cost_coverage(config.cost.as_ref(), cost_summary) {
        analysis.violations.push(violation);
    }
    analysis.passed = analysis.violations.is_empty();
    analysis
}

fn evaluate_cost_coverage(
    cost_config: Option<&costguard_cost::CostConfig>,
    summary: Option<&ProjectCostSummary>,
) -> Option<AnalysisViolation> {
    let cost_config = cost_config.filter(|config| config.enabled)?;
    let threshold = cost_config.min_mapped_spend_fraction?;
    let summary = summary?;
    if summary.coverage.models_total == 0 {
        return None;
    }
    let observed = summary.coverage.mapped_spend_fraction;
    if observed >= threshold {
        return None;
    }
    Some(AnalysisViolation {
        code: "cost_coverage".into(),
        message: format!(
            "mapped spend coverage {:.0}% is below required {:.0}% ({}/{} models with measured spend)",
            observed * 100.0,
            threshold * 100.0,
            summary.coverage.models_with_mapped_spend,
            summary.coverage.models_total
        ),
        observed,
        allowed: threshold,
    })
}

fn evaluate_analysis(
    config: &ScanConfig,
    metrics: &ScanMetrics,
    target_skipped_files: usize,
    metadata_only: bool,
    manifest_stale: bool,
    pr_mode: bool,
    context: Option<&ContextReport>,
) -> AnalysisReport {
    let analysis = config.analysis.effective();
    let mut violations = Vec::new();
    let parse_total = metrics.sql_parse_total + metrics.sql_parse_other_total;
    let parse_failures = metrics.sql_parse_failures + metrics.sql_parse_other_failures;
    let parse_failure_rate = if parse_total == 0 {
        0.0
    } else {
        parse_failures as f64 / parse_total as f64
    };
    if parse_failure_rate > analysis.max_parse_failure_rate {
        violations.push(AnalysisViolation {
            code: "parse_failure_rate".into(),
            message: format!(
                "SQL parse failure rate {:.2}% exceeds allowed {:.2}%",
                parse_failure_rate * 100.0,
                analysis.max_parse_failure_rate * 100.0
            ),
            observed: parse_failure_rate,
            allowed: analysis.max_parse_failure_rate,
        });
    }
    if metrics.sql_parse_compiled_failures > analysis.max_compiled_parse_failures {
        violations.push(AnalysisViolation {
            code: "compiled_parse_failures".into(),
            message: format!(
                "{} compiled SQL parse failures exceed allowed {}",
                metrics.sql_parse_compiled_failures, analysis.max_compiled_parse_failures
            ),
            observed: metrics.sql_parse_compiled_failures as f64,
            allowed: analysis.max_compiled_parse_failures as f64,
        });
    }
    if target_skipped_files > analysis.max_skipped_files {
        violations.push(AnalysisViolation {
            code: "skipped_files".into(),
            message: format!(
                "{target_skipped_files} skipped or unreadable target files exceed allowed {}",
                analysis.max_skipped_files
            ),
            observed: target_skipped_files as f64,
            allowed: analysis.max_skipped_files as f64,
        });
    }
    let metadata_errors = metrics.yaml_parse_failures + metrics.dbt_project_parse_failures;
    if analysis.fail_on_metadata_errors && metadata_errors > 0 {
        violations.push(AnalysisViolation {
            code: "metadata_errors".into(),
            message: format!("{metadata_errors} metadata parse errors make analysis incomplete"),
            observed: metadata_errors as f64,
            allowed: 0.0,
        });
    }
    if analysis.require_manifest && metadata_only {
        violations.push(AnalysisViolation {
            code: "manifest_required".into(),
            message: "a dbt manifest is required for this analysis policy; run 'dbt compile' to produce target/manifest.json or pass --manifest <path>".into(),
            observed: 0.0,
            allowed: 1.0,
        });
    }
    if analysis.require_manifest && manifest_stale {
        violations.push(AnalysisViolation {
            code: "manifest_stale".into(),
            message: "dbt manifest is stale (older than modified models); run 'dbt compile' before scanning".into(),
            observed: 1.0,
            allowed: 0.0,
        });
    }
    if pr_mode {
        let _ = context;
    }
    AnalysisReport {
        policy: analysis.policy,
        passed: violations.is_empty(),
        violations,
    }
}
