use crate::sign::parse_time;
use crate::types::{PolicyException, PolicyViolation, ResolvedPolicy};
use anyhow::Result;
use chrono::{DateTime, Utc};
use costguard_diagnostics::Diagnostic;
use costguard_protocol::{
    AppliedExceptionV1, EnforcementMode, EnforcementOutcome, PolicyProvenanceV1,
};
use globset::Glob;

pub fn apply_governance(
    diagnostics: &mut [Diagnostic],
    policy: &ResolvedPolicy,
    repository: &str,
    now: DateTime<Utc>,
) -> Result<Vec<PolicyViolation>> {
    let provenance = PolicyProvenanceV1 {
        digest: policy.digest.clone(),
        version: policy.document.version.clone(),
        scope: policy.scope_ids.join(","),
    };
    let mut violations = Vec::new();
    for diagnostic in diagnostics {
        diagnostic.governance.policy = Some(provenance.clone());
        let enforcement = policy
            .rules
            .get(&diagnostic.rule_id)
            .and_then(|rule| rule.enforcement)
            .unwrap_or(policy.enforcement);
        diagnostic.governance.enforcement = outcome(enforcement);
        if let Some(exception) = policy
            .exceptions
            .iter()
            .find(|exception| exception_matches(exception, diagnostic, repository).unwrap_or(false))
        {
            let expires = parse_time("exception.expires_at", &exception.expires_at)?;
            if now <= expires {
                diagnostic.governance.enforcement = EnforcementOutcome::Excepted;
                diagnostic.governance.exception = Some(AppliedExceptionV1 {
                    id: exception.id.clone(),
                    owner: exception.owner.clone(),
                    reason: exception.reason.clone(),
                    ticket_url: exception.ticket_url.clone(),
                    approver: exception.approver.clone(),
                    expires_at: exception.expires_at.clone(),
                });
            } else {
                violations.push(PolicyViolation {
                    code: "expired_exception".into(),
                    message: format!(
                        "exception '{}' expired at {}",
                        exception.id, exception.expires_at
                    ),
                });
            }
        }
    }
    Ok(violations)
}

fn exception_matches(
    exception: &PolicyException,
    diagnostic: &Diagnostic,
    repository: &str,
) -> Result<bool> {
    let repository_match = Glob::new(&exception.repository)?
        .compile_matcher()
        .is_match(repository);
    let path = costguard_diagnostics::posix_path(&diagnostic.path);
    let path_match = Glob::new(&exception.path)?.compile_matcher().is_match(path);
    let finding_match = exception
        .finding_id
        .as_deref()
        .is_none_or(|id| id == diagnostic.governance.finding_id);
    let rule_match = exception
        .rule_id
        .as_deref()
        .is_none_or(|id| id.eq_ignore_ascii_case(&diagnostic.rule_id));
    Ok(repository_match && path_match && finding_match && rule_match)
}

fn outcome(mode: EnforcementMode) -> EnforcementOutcome {
    match mode {
        EnforcementMode::Observe => EnforcementOutcome::Observed,
        EnforcementMode::Warn => EnforcementOutcome::Warned,
        EnforcementMode::Block => EnforcementOutcome::Blocked,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resolve::resolve_policy;
    use crate::types::{
        PolicyDocumentV2, PolicyException, PolicyPermissions, PolicyScope, ResolutionContext,
        ScopeKind,
    };
    use chrono::Utc;
    use costguard_diagnostics::Severity;
    use costguard_protocol::{EnforcementMode, IDENTITY_SCHEME_SEMANTIC_V1};
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    fn policy(now: DateTime<Utc>) -> PolicyDocumentV2 {
        PolicyDocumentV2 {
            schema_version: costguard_protocol::POLICY_SCHEMA_VERSION,
            id: "org-default".into(),
            version: "2026.06".into(),
            organization: "acme".into(),
            issued_at: now.to_rfc3339(),
            expires_at: (now + chrono::Duration::days(30)).to_rfc3339(),
            identity_scheme: IDENTITY_SCHEME_SEMANTIC_V1.into(),
            permissions: PolicyPermissions::default(),
            scopes: vec![PolicyScope {
                id: "org".into(),
                kind: ScopeKind::Organization,
                selector: "acme".into(),
                priority: 0,
                enforcement: EnforcementMode::Block,
                rules: BTreeMap::new(),
                custom_rules: vec![],
                exceptions: vec![],
            }],
        }
    }

    #[test]
    fn expired_exception_does_not_suppress() {
        let now = Utc::now();
        let mut resolved = resolve_policy(
            &policy(now),
            &ResolutionContext {
                organization: "acme",
                repository: "warehouse",
                ..ResolutionContext::default()
            },
        )
        .unwrap();
        resolved.exceptions.push(PolicyException {
            id: "exc-1".into(),
            finding_id: None,
            rule_id: Some("SQLCOST001".into()),
            repository: "warehouse".into(),
            path: "models/**".into(),
            owner: "data-platform".into(),
            reason: "migration".into(),
            ticket_url: "https://tickets.example/1".into(),
            approver: "admin".into(),
            created_at: (now - chrono::Duration::days(2)).to_rfc3339(),
            expires_at: (now - chrono::Duration::days(1)).to_rfc3339(),
        });
        let mut diagnostic = Diagnostic::new(
            "SQLCOST001",
            Severity::High,
            PathBuf::from("models/a.sql"),
            None,
            "test",
        );
        diagnostic.assign_identity("primary");
        let violations = apply_governance(&mut [diagnostic], &resolved, "warehouse", now).unwrap();
        assert_eq!(violations[0].code, "expired_exception");
    }
}
