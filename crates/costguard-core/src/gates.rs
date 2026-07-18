use crate::config::{GateConfig, GateMode, GateScope};
use crate::{
    ChangedModelDetail, FindingDelta, GateResult, GateStatus, PrSummary, ProjectCostSummary,
};
use costguard_diagnostics::{posix_path, Confidence, Diagnostic, Severity};
use costguard_protocol::EnforcementOutcome;
use globset::Glob;
use std::collections::HashSet;

struct GateCriteria<'a> {
    name: &'a str,
    mode: GateMode,
    paths: &'a [String],
    tags: &'a [String],
    owners: &'a [String],
    fail_on: Option<Severity>,
    min_confidence: Option<Confidence>,
    fail_on_monthly_delta: Option<f64>,
    fail_on_monthly_delta_gb: Option<f64>,
    fail_on_pr_cost_increase: Option<f64>,
    fail_on_blast_radius: Option<usize>,
    require_owner: bool,
}

pub(crate) fn evaluate_gates(
    config: Option<&GateConfig>,
    diagnostics: &[Diagnostic],
    summary: &PrSummary,
    cost_summary: Option<&ProjectCostSummary>,
) -> Vec<GateResult> {
    let Some(config) = config else {
        return Vec::new();
    };
    let global = GateCriteria {
        name: config.name.as_deref().unwrap_or("default"),
        mode: config.mode,
        paths: &[],
        tags: &[],
        owners: &[],
        fail_on: config.fail_on,
        min_confidence: config.min_confidence,
        fail_on_monthly_delta: config.fail_on_monthly_delta,
        fail_on_monthly_delta_gb: config.fail_on_monthly_delta_gb,
        fail_on_pr_cost_increase: config.fail_on_pr_cost_increase,
        fail_on_blast_radius: config.fail_on_blast_radius,
        require_owner: config.require_owner,
    };
    let mut results = vec![evaluate_gate(
        &global,
        diagnostics,
        summary,
        cost_summary,
        config.block_only_new,
        summary.finding_delta.as_ref(),
    )];
    results.extend(config.scopes.iter().map(|scope| {
        let criteria = criteria_for_scope(scope);
        evaluate_gate(
            &criteria,
            diagnostics,
            summary,
            cost_summary,
            config.block_only_new,
            summary.finding_delta.as_ref(),
        )
    }));
    results
}

fn criteria_for_scope(scope: &GateScope) -> GateCriteria<'_> {
    GateCriteria {
        name: &scope.name,
        mode: scope.mode,
        paths: &scope.paths,
        tags: &scope.tags,
        owners: &scope.owners,
        fail_on: scope.fail_on,
        min_confidence: scope.min_confidence,
        fail_on_monthly_delta: scope.fail_on_monthly_delta,
        fail_on_monthly_delta_gb: scope.fail_on_monthly_delta_gb,
        fail_on_pr_cost_increase: None,
        fail_on_blast_radius: scope.fail_on_blast_radius,
        require_owner: scope.require_owner,
    }
}

fn evaluate_gate(
    gate: &GateCriteria<'_>,
    diagnostics: &[Diagnostic],
    summary: &PrSummary,
    cost_summary: Option<&ProjectCostSummary>,
    block_only_new: bool,
    finding_delta: Option<&FindingDelta>,
) -> GateResult {
    let models = summary
        .changed_model_details
        .iter()
        .filter(|detail| model_matches(gate, detail))
        .collect::<Vec<_>>();
    let actionable = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.governance.enforcement != EnforcementOutcome::Excepted)
        .filter(|diagnostic| !costguard_cost::is_infrastructure_rule(&diagnostic.rule_id))
        .filter(|diagnostic| {
            !block_only_new
                || finding_delta
                    .is_none_or(|delta| delta.is_blocking(&diagnostic.governance.finding_id))
        })
        .filter(|diagnostic| diagnostic_matches(gate, diagnostic, summary))
        .collect::<Vec<_>>();
    let mut reasons = Vec::new();

    if let Some(threshold) = gate.fail_on {
        let count = actionable
            .iter()
            .filter(|diagnostic| {
                diagnostic.severity >= threshold
                    && gate
                        .min_confidence
                        .is_none_or(|minimum| diagnostic.confidence >= minimum)
            })
            .count();
        if count > 0 {
            reasons.push(format!(
                "{count} finding(s) at or above {} severity",
                threshold.label().to_ascii_lowercase()
            ));
        }
    }

    let monthly = actionable
        .iter()
        .filter_map(|diagnostic| {
            diagnostic
                .cost_estimate
                .as_ref()
                .and_then(|cost| cost.savings_p50_usd_per_month)
        })
        .sum::<f64>();
    if gate
        .fail_on_monthly_delta
        .is_some_and(|threshold| monthly >= threshold)
    {
        reasons.push(format!(
            "estimated monthly savings ${monthly:.0} meets cost threshold"
        ));
    }

    if let Some(threshold) = gate.fail_on_pr_cost_increase {
        match cost_summary
            .and_then(|summary| summary.pr_impact.as_ref())
            .and_then(|impact| impact.net.monthly_p50)
        {
            Some(net) if net >= threshold => reasons.push(format!(
                "estimated PR cost increase ${net:.0}/mo meets ${threshold:.0}/mo threshold"
            )),
            Some(_) => {}
            None => reasons.push("PR cost impact unavailable; cannot evaluate cost gate".into()),
        }
    }

    let monthly_gb = actionable
        .iter()
        .filter_map(|diagnostic| diagnostic.cost_estimate.as_ref())
        .map(|cost| cost.relative_index)
        .sum::<f64>();
    if gate
        .fail_on_monthly_delta_gb
        .is_some_and(|threshold| monthly_gb >= threshold)
    {
        reasons.push(format!(
            "estimated monthly savings {monthly_gb:.0} GB-mo meets volume threshold"
        ));
    }

    let blast_radius = models
        .iter()
        .flat_map(|model| model.affected_downstream.iter())
        .collect::<HashSet<_>>()
        .len();
    if gate
        .fail_on_blast_radius
        .is_some_and(|threshold| blast_radius >= threshold)
    {
        reasons.push(format!(
            "{blast_radius} downstream model(s) meet blast-radius threshold"
        ));
    }

    if gate.require_owner {
        let unowned_models = models
            .iter()
            .filter(|model| model.owners.is_empty())
            .count();
        let model_paths = models
            .iter()
            .map(|model| &model.path)
            .collect::<HashSet<_>>();
        let changed_paths = summary.changed_files.iter().collect::<HashSet<_>>();
        let unowned_findings = actionable
            .iter()
            .filter(|diagnostic| {
                diagnostic.governance.owners.is_empty()
                    && changed_paths.contains(&diagnostic.path)
                    && !model_paths.contains(&diagnostic.path)
            })
            .count();
        let unowned = unowned_models + unowned_findings;
        if unowned > 0 {
            reasons.push(format!(
                "{unowned} changed model/finding path(s) have no owner"
            ));
        }
    }

    let status = if reasons.is_empty() {
        GateStatus::Pass
    } else if gate.mode == GateMode::Warn {
        GateStatus::Warn
    } else {
        GateStatus::Fail
    };
    GateResult {
        name: gate.name.to_string(),
        mode: gate.mode,
        status,
        matched_findings: actionable.len(),
        reasons,
    }
}

fn diagnostic_matches(
    gate: &GateCriteria<'_>,
    diagnostic: &Diagnostic,
    summary: &PrSummary,
) -> bool {
    if gate.paths.is_empty() && gate.tags.is_empty() && gate.owners.is_empty() {
        return true;
    }
    let detail = summary
        .changed_model_details
        .iter()
        .find(|detail| detail.path == diagnostic.path);
    match_fields(
        gate,
        &posix_path(&diagnostic.path),
        detail.map(|detail| detail.tags.as_slice()).unwrap_or(&[]),
        &diagnostic.governance.owners,
    )
}

fn model_matches(gate: &GateCriteria<'_>, detail: &ChangedModelDetail) -> bool {
    match_fields(
        gate,
        &posix_path(&detail.path),
        &detail.tags,
        &detail.owners,
    )
}

fn match_fields(gate: &GateCriteria<'_>, path: &str, tags: &[String], owners: &[String]) -> bool {
    (gate.paths.is_empty()
        || gate.paths.iter().any(|pattern| {
            Glob::new(pattern)
                .expect("gate globs are validated before scanning")
                .compile_matcher()
                .is_match(path)
        }))
        && (gate.tags.is_empty() || gate.tags.iter().any(|tag| tags.contains(tag)))
        && (gate.owners.is_empty() || gate.owners.iter().any(|owner| owners.contains(owner)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ChangedModelDetail, PrSummary};
    use costguard_cost::{CostFigure, PrCostImpact, ProjectCostSummary};
    use costguard_diagnostics::{CostEstimate, CostGrade, Diagnostic, Severity};
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    #[test]
    fn scoped_gate_blocks_actionable_owned_path_and_warn_mode_does_not_fail() {
        let mut diagnostic = Diagnostic::new(
            "SQLCOST001",
            Severity::High,
            PathBuf::from("models/finance/orders.sql"),
            None,
            "test",
        );
        diagnostic.governance.owners = vec!["@finance".into()];
        let detail = ChangedModelDetail {
            id: "model.pkg.orders".into(),
            name: "orders".into(),
            framework: Some("dbt".into()),
            path: diagnostic.path.clone(),
            tags: vec!["critical".into()],
            owners: diagnostic.governance.owners.clone(),
            group: None,
            affected_downstream: vec!["report".into()],
            affected_exposures: Vec::new(),
            affected_exposure_owners: BTreeMap::new(),
        };
        let config = GateConfig {
            scopes: vec![GateScope {
                name: "finance".into(),
                paths: vec!["models/finance/**".into()],
                owners: vec!["@finance".into()],
                fail_on: Some(Severity::High),
                ..GateScope::default()
            }],
            ..GateConfig::default()
        };
        let summary = PrSummary {
            changed_model_details: vec![detail],
            ..PrSummary::default()
        };
        let results = evaluate_gates(Some(&config), &[diagnostic], &summary, None);
        assert_eq!(results[1].status, GateStatus::Fail);
    }

    fn identified_diagnostic(id: &str, savings: Option<f64>) -> Diagnostic {
        let mut diagnostic = Diagnostic::new(
            "SQLCOST001",
            Severity::High,
            PathBuf::from("models/a.sql"),
            None,
            "test",
        );
        diagnostic.assign_identity(format!("sem-v1:{id}"));
        diagnostic.cost_estimate = savings.map(|value| CostEstimate {
            relative_index: value,
            grade: CostGrade::B,
            basis: "test".into(),
            prior_basis: None,
            currency: "USD".into(),
            model_id: None,
            model_monthly_p50_usd: None,
            savings_p10_usd_per_month: Some(value),
            savings_p50_usd_per_month: Some(value),
            savings_p90_usd_per_month: Some(value),
            current_cost_p50_usd_per_month: None,
            post_fix_cost_p50_usd_per_month: None,
            unestimated_reason: None,
            downstream_model_count: None,
            downstream_monthly_p50_usd: None,
        });
        diagnostic
    }

    #[test]
    fn regression_only_filters_unchanged_severity_and_savings() {
        let unchanged = identified_diagnostic("unchanged", Some(500.0));
        let introduced = identified_diagnostic("introduced", Some(200.0));
        let config = GateConfig {
            block_only_new: true,
            fail_on: Some(Severity::High),
            fail_on_monthly_delta: Some(300.0),
            ..GateConfig::default()
        };
        let summary = PrSummary {
            finding_delta: Some(FindingDelta {
                introduced: 1,
                unchanged: 1,
                introduced_ids: vec![introduced.governance.finding_id.clone()],
                ..FindingDelta::default()
            }),
            ..PrSummary::default()
        };

        let results = evaluate_gates(Some(&config), &[unchanged, introduced], &summary, None);
        assert_eq!(results[0].status, GateStatus::Fail);
        assert_eq!(results[0].matched_findings, 1);
        assert_eq!(results[0].reasons.len(), 1);
        assert!(results[0].reasons[0].contains("severity"));
    }

    #[test]
    fn regression_only_scoped_savings_exclude_unchanged_findings() {
        let unchanged = identified_diagnostic("unchanged", Some(500.0));
        let introduced = identified_diagnostic("introduced", Some(200.0));
        let config = GateConfig {
            block_only_new: true,
            scopes: vec![GateScope {
                name: "models".into(),
                paths: vec!["models/**".into()],
                fail_on_monthly_delta: Some(300.0),
                ..GateScope::default()
            }],
            ..GateConfig::default()
        };
        let summary = PrSummary {
            finding_delta: Some(FindingDelta {
                introduced: 1,
                unchanged: 1,
                introduced_ids: vec![introduced.governance.finding_id.clone()],
                ..FindingDelta::default()
            }),
            ..PrSummary::default()
        };
        let results = evaluate_gates(Some(&config), &[unchanged, introduced], &summary, None);
        assert_eq!(results[1].status, GateStatus::Pass);
        assert_eq!(results[1].matched_findings, 1);
    }

    #[test]
    fn regression_only_unchanged_passes_and_missing_identity_fails_closed() {
        let unchanged = identified_diagnostic("unchanged", None);
        let config = GateConfig {
            block_only_new: true,
            fail_on: Some(Severity::High),
            ..GateConfig::default()
        };
        let summary = PrSummary {
            finding_delta: Some(FindingDelta {
                unchanged: 1,
                ..FindingDelta::default()
            }),
            ..PrSummary::default()
        };
        let pass = evaluate_gates(Some(&config), &[unchanged], &summary, None);
        assert_eq!(pass[0].status, GateStatus::Pass);

        let unidentified = Diagnostic::new(
            "SQLCOST001",
            Severity::High,
            PathBuf::from("models/a.sql"),
            None,
            "test",
        );
        let fail = evaluate_gates(Some(&config), &[unidentified], &summary, None);
        assert_eq!(fail[0].status, GateStatus::Fail);

        let infrastructure = Diagnostic::new(
            "SQLCOST023",
            Severity::High,
            PathBuf::from("."),
            None,
            "manifest missing",
        );
        let excluded = evaluate_gates(Some(&config), &[infrastructure], &summary, None);
        assert_eq!(excluded[0].status, GateStatus::Pass);
        assert_eq!(excluded[0].matched_findings, 0);
    }

    #[test]
    fn net_pr_cost_gate_uses_project_impact_and_fails_closed() {
        let config = GateConfig {
            fail_on_pr_cost_increase: Some(100.0),
            ..GateConfig::default()
        };
        let summary = PrSummary::default();
        let cost = |net: f64| ProjectCostSummary {
            pr_impact: Some(PrCostImpact {
                net: CostFigure {
                    monthly_p50: Some(net),
                    ..CostFigure::default()
                },
                ..PrCostImpact::default()
            }),
            ..ProjectCostSummary::default()
        };

        assert_eq!(
            evaluate_gates(Some(&config), &[], &summary, Some(&cost(99.0)))[0].status,
            GateStatus::Pass
        );
        assert_eq!(
            evaluate_gates(Some(&config), &[], &summary, Some(&cost(100.0)))[0].status,
            GateStatus::Fail
        );
        assert_eq!(
            evaluate_gates(Some(&config), &[], &summary, Some(&cost(-100.0)))[0].status,
            GateStatus::Pass
        );
        let missing = evaluate_gates(Some(&config), &[], &summary, None);
        assert_eq!(missing[0].status, GateStatus::Fail);
        assert!(missing[0].reasons[0].contains("unavailable"));
    }

    #[test]
    fn model_change_controls_ignore_finding_regression_filter() {
        let config = GateConfig {
            block_only_new: true,
            require_owner: true,
            fail_on_blast_radius: Some(1),
            ..GateConfig::default()
        };
        let summary = PrSummary {
            changed_model_details: vec![ChangedModelDetail {
                id: "model.pkg.a".into(),
                name: "a".into(),
                framework: Some("dbt".into()),
                path: PathBuf::from("models/a.sql"),
                tags: Vec::new(),
                owners: Vec::new(),
                group: None,
                affected_downstream: vec!["model.pkg.b".into()],
                affected_exposures: Vec::new(),
                affected_exposure_owners: BTreeMap::new(),
            }],
            finding_delta: Some(FindingDelta::default()),
            ..PrSummary::default()
        };
        let results = evaluate_gates(Some(&config), &[], &summary, None);
        assert_eq!(results[0].status, GateStatus::Fail);
        assert_eq!(results[0].reasons.len(), 2);
    }
}
