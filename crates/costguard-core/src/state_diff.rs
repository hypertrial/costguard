use costguard_diagnostics::Diagnostic;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct FindingDelta {
    pub introduced: usize,
    pub regressed: usize,
    pub resolved: usize,
    pub unchanged: usize,
    pub introduced_ids: Vec<String>,
    pub regressed_ids: Vec<String>,
}

pub fn classify_findings(head: &[Diagnostic], base: &[Diagnostic]) -> FindingDelta {
    let base_by_id = index_by_finding_id(base);
    let head_by_id = index_by_finding_id(head);
    let mut introduced_ids = Vec::new();
    let mut regressed_ids = Vec::new();
    let mut resolved = 0usize;
    let mut unchanged = 0usize;

    for (finding_id, head_diagnostic) in &head_by_id {
        match base_by_id.get(finding_id) {
            None => introduced_ids.push(finding_id.clone()),
            Some(base_diagnostic) => {
                if is_regression(base_diagnostic, head_diagnostic) {
                    regressed_ids.push(finding_id.clone());
                } else {
                    unchanged += 1;
                }
            }
        }
    }
    for finding_id in base_by_id.keys() {
        if !head_by_id.contains_key(finding_id) {
            resolved += 1;
        }
    }

    introduced_ids.sort();
    regressed_ids.sort();
    FindingDelta {
        introduced: introduced_ids.len(),
        regressed: regressed_ids.len(),
        resolved,
        unchanged,
        introduced_ids,
        regressed_ids,
    }
}

fn index_by_finding_id(diagnostics: &[Diagnostic]) -> BTreeMap<String, &Diagnostic> {
    diagnostics
        .iter()
        .filter(|diagnostic| !diagnostic.governance.finding_id.is_empty())
        .map(|diagnostic| (diagnostic.governance.finding_id.clone(), diagnostic))
        .collect()
}

fn is_regression(base: &Diagnostic, head: &Diagnostic) -> bool {
    if head.severity > base.severity {
        return true;
    }
    let base_cost = cost_signal(base);
    let head_cost = cost_signal(head);
    head_cost > base_cost + f64::EPSILON
}

fn cost_signal(diagnostic: &Diagnostic) -> f64 {
    diagnostic
        .cost_estimate
        .as_ref()
        .and_then(|estimate| {
            estimate
                .savings_p50_usd_per_month
                .or(Some(estimate.relative_index))
        })
        .unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use costguard_diagnostics::{Confidence, CostEstimate, CostGrade, Severity};
    use std::path::PathBuf;

    fn diagnostic(evidence_suffix: &str, severity: Severity, savings: Option<f64>) -> Diagnostic {
        let evidence_key = format!("sem-v1:test:{evidence_suffix}");
        let mut diagnostic = Diagnostic {
            governance: costguard_diagnostics::DiagnosticGovernance {
                evidence_key: evidence_key.clone(),
                ..Default::default()
            },
            rule_id: "SQLCOST001".into(),
            severity,
            path: PathBuf::from("models/a.sql"),
            line: 1,
            column: 1,
            span: None,
            message: "test".into(),
            risk: None,
            suggestion: None,
            confidence: Confidence::High,
            warehouse: None,
            source_provenance: None,
            compiled_line: None,
            compiled_column: None,
            cost_estimate: savings.map(|relative_index| CostEstimate {
                relative_index,
                grade: CostGrade::B,
                basis: "test".into(),
                prior_basis: None,
                currency: "USD".into(),
                model_id: None,
                model_monthly_p50_usd: None,
                savings_p10_usd_per_month: None,
                savings_p50_usd_per_month: Some(relative_index),
                savings_p90_usd_per_month: None,
                current_cost_p50_usd_per_month: None,
                post_fix_cost_p50_usd_per_month: None,
                unestimated_reason: None,
                downstream_model_count: None,
                downstream_monthly_p50_usd: None,
            }),
            rule_precision_tier: None,
        };
        diagnostic.assign_identity(evidence_key);
        diagnostic
    }

    #[test]
    fn classifies_introduced_regressed_resolved_and_unchanged() {
        let base = vec![
            diagnostic("regressed", Severity::High, Some(10.0)),
            diagnostic("resolved", Severity::Medium, None),
            diagnostic("unchanged", Severity::Low, Some(1.0)),
        ];
        let head = vec![
            diagnostic("regressed", Severity::Critical, Some(10.0)),
            diagnostic("introduced", Severity::High, None),
            diagnostic("unchanged", Severity::Low, Some(1.0)),
        ];

        let delta = classify_findings(&head, &base);
        assert_eq!(delta.introduced, 1);
        assert_eq!(delta.regressed, 1);
        assert_eq!(delta.resolved, 1);
        assert_eq!(delta.unchanged, 1);
        assert_eq!(delta.introduced_ids.len(), 1);
        assert_eq!(delta.regressed_ids.len(), 1);
    }

    #[test]
    fn deterministic_cost_increase_is_a_regression() {
        let base = vec![diagnostic("cost", Severity::Medium, Some(10.0))];
        let head = vec![diagnostic("cost", Severity::Medium, Some(10.5))];
        let delta = classify_findings(&head, &base);
        assert_eq!(delta.regressed, 1);
        assert_eq!(delta.unchanged, 0);
    }
}
