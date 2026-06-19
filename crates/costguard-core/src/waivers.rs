use crate::{AnalysisViolation, Waiver};
use costguard_diagnostics::Diagnostic;
use costguard_protocol::{AppliedExceptionV1, EnforcementOutcome};
use globset::Glob;

pub(crate) fn apply_local_waivers(
    diagnostics: &mut [Diagnostic],
    waivers: &[Waiver],
    now: chrono::DateTime<chrono::Utc>,
) -> Vec<AnalysisViolation> {
    let mut violations = Vec::new();
    for waiver in waivers {
        let expires = chrono::DateTime::parse_from_rfc3339(&waiver.expires_at)
            .expect("waivers are validated before scanning")
            .with_timezone(&chrono::Utc);
        if now > expires {
            violations.push(AnalysisViolation {
                code: "expired_waiver".into(),
                message: format!("waiver '{}' expired at {}", waiver.id, waiver.expires_at),
                observed: 1.0,
                allowed: 0.0,
            });
            continue;
        }
        let matcher = Glob::new(&waiver.path)
            .expect("waivers are validated before scanning")
            .compile_matcher();
        for diagnostic in diagnostics.iter_mut().filter(|diagnostic| {
            matcher.is_match(costguard_diagnostics::posix_path(&diagnostic.path))
                && waiver
                    .finding_id
                    .as_deref()
                    .is_none_or(|finding_id| diagnostic.governance.finding_id == finding_id)
                && waiver
                    .rule_id
                    .as_deref()
                    .is_none_or(|rule_id| diagnostic.rule_id.eq_ignore_ascii_case(rule_id))
        }) {
            if diagnostic.governance.enforcement == EnforcementOutcome::Excepted {
                continue;
            }
            diagnostic.governance.enforcement = EnforcementOutcome::Excepted;
            diagnostic.governance.exception = Some(AppliedExceptionV1 {
                id: waiver.id.clone(),
                owner: waiver.owner.clone(),
                reason: waiver.reason.clone(),
                ticket_url: waiver.ticket_url.clone(),
                approver: waiver.approver.clone(),
                expires_at: waiver.expires_at.clone(),
            });
        }
    }
    violations
}

#[cfg(test)]
mod tests {
    use super::*;
    use costguard_diagnostics::{Diagnostic, Severity};
    use std::path::PathBuf;

    fn waiver(expires_at: &str) -> Waiver {
        Waiver {
            id: "CG-1".into(),
            finding_id: None,
            rule_id: Some("SQLCOST001".into()),
            path: "models/**".into(),
            owner: "@data".into(),
            reason: "accepted migration cost".into(),
            ticket_url: "https://example.com/CG-1".into(),
            approver: "@lead".into(),
            created_at: "2026-01-01T00:00:00Z".into(),
            expires_at: expires_at.into(),
        }
    }

    #[test]
    fn active_waiver_excepts_match_and_expired_waiver_violates() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-06-01T00:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let mut diagnostic = Diagnostic::new(
            "SQLCOST001",
            Severity::High,
            PathBuf::from("models/orders.sql"),
            None,
            "test",
        );
        diagnostic.governance.finding_id = "finding-1".into();
        let mut diagnostics = vec![diagnostic];
        assert!(
            apply_local_waivers(&mut diagnostics, &[waiver("2026-07-01T00:00:00Z")], now)
                .is_empty()
        );
        assert_eq!(
            diagnostics[0].governance.enforcement,
            EnforcementOutcome::Excepted
        );

        let violations =
            apply_local_waivers(&mut diagnostics, &[waiver("2026-05-01T00:00:00Z")], now);
        assert_eq!(violations[0].code, "expired_waiver");
    }
}
