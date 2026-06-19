use crate::config::{GateConfig, GateMode, GateScope};
use crate::{ChangedModelDetail, GateResult, GateStatus, PrSummary};
use costguard_diagnostics::{Confidence, Diagnostic, Severity};
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
    fail_on_blast_radius: Option<usize>,
    require_owner: bool,
}

pub(crate) fn evaluate_gates(
    config: Option<&GateConfig>,
    diagnostics: &[Diagnostic],
    summary: &PrSummary,
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
        fail_on_blast_radius: config.fail_on_blast_radius,
        require_owner: config.require_owner,
    };
    let mut results = vec![evaluate_gate(&global, diagnostics, summary)];
    results.extend(config.scopes.iter().map(|scope| {
        let criteria = criteria_for_scope(scope);
        evaluate_gate(&criteria, diagnostics, summary)
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
        fail_on_blast_radius: scope.fail_on_blast_radius,
        require_owner: scope.require_owner,
    }
}

fn evaluate_gate(
    gate: &GateCriteria<'_>,
    diagnostics: &[Diagnostic],
    summary: &PrSummary,
) -> GateResult {
    let models = summary
        .changed_model_details
        .iter()
        .filter(|detail| model_matches(gate, detail))
        .collect::<Vec<_>>();
    let actionable = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.governance.enforcement != EnforcementOutcome::Excepted)
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
        &diagnostic.path.to_string_lossy().replace('\\', "/"),
        detail.map(|detail| detail.tags.as_slice()).unwrap_or(&[]),
        &diagnostic.governance.owners,
    )
}

fn model_matches(gate: &GateCriteria<'_>, detail: &ChangedModelDetail) -> bool {
    match_fields(
        gate,
        &detail.path.to_string_lossy().replace('\\', "/"),
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
    use costguard_diagnostics::{Diagnostic, Severity};
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
        let results = evaluate_gates(Some(&config), &[diagnostic], &summary);
        assert_eq!(results[1].status, GateStatus::Fail);
    }
}
