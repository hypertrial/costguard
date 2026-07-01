use crate::compile::validate_policy;
use crate::types::{PolicyDocumentV2, ResolutionContext, ResolvedPolicy};
use anyhow::Result;
use costguard_protocol::EnforcementMode;
use globset::Glob;

pub fn resolve_policy(
    policy: &PolicyDocumentV2,
    context: &ResolutionContext<'_>,
) -> Result<ResolvedPolicy> {
    validate_policy(policy)?;
    if policy.organization != context.organization {
        anyhow::bail!(
            "policy organization '{}' does not match '{}'",
            policy.organization,
            context.organization
        );
    }
    let mut matching = policy
        .scopes
        .iter()
        .filter(|scope| scope_matches(scope, context).unwrap_or(false))
        .collect::<Vec<_>>();
    matching.sort_by_key(|scope| (scope.kind.specificity(), scope.priority));

    for pair in matching.windows(2) {
        if pair[0].kind == pair[1].kind
            && pair[0].priority == pair[1].priority
            && scopes_conflict(pair[0], pair[1])
        {
            anyhow::bail!(
                "matching policy scopes '{}' and '{}' conflict at equal specificity and priority",
                pair[0].id,
                pair[1].id
            );
        }
    }

    let mut enforcement = EnforcementMode::Observe;
    let mut rules = std::collections::BTreeMap::new();
    let mut custom_rules = Vec::new();
    let mut exceptions = Vec::new();
    let mut scope_ids = Vec::new();
    for scope in matching {
        enforcement = scope.enforcement;
        scope_ids.push(scope.id.clone());
        rules.extend(scope.rules.clone());
        custom_rules.extend(scope.custom_rules.clone());
        exceptions.extend(scope.exceptions.clone());
    }
    Ok(ResolvedPolicy {
        document: policy.clone(),
        digest: crate::policy_digest(policy)?,
        scope_ids,
        enforcement,
        rules,
        custom_rules,
        exceptions,
    })
}

fn scope_matches(
    scope: &crate::types::PolicyScope,
    context: &ResolutionContext<'_>,
) -> Result<bool> {
    use crate::types::ScopeKind;
    Ok(match scope.kind {
        ScopeKind::Organization => scope.selector == context.organization,
        ScopeKind::Team => context.team == Some(scope.selector.as_str()),
        ScopeKind::Repository => scope.selector == context.repository,
        ScopeKind::Path => context.path.is_some_and(|path| {
            Glob::new(&scope.selector)
                .map(|glob| glob.compile_matcher().is_match(path))
                .unwrap_or(false)
        }),
    })
}

fn scopes_conflict(left: &crate::types::PolicyScope, right: &crate::types::PolicyScope) -> bool {
    left.enforcement != right.enforcement
        || left
            .rules
            .iter()
            .any(|(id, rule)| right.rules.get(id).is_some_and(|other| other != rule))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{PolicyDocumentV2, PolicyPermissions, PolicyScope, RulePolicy, ScopeKind};
    use chrono::Utc;
    use costguard_diagnostics::Severity;
    use costguard_protocol::{EnforcementMode, IDENTITY_SCHEME_SEMANTIC_V1};
    use std::collections::BTreeMap;

    fn policy(now: chrono::DateTime<Utc>) -> PolicyDocumentV2 {
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
    fn more_specific_scopes_override_broader_scopes() {
        let now = Utc::now();
        let mut document = policy(now);
        document.scopes.push(PolicyScope {
            id: "repository".into(),
            kind: ScopeKind::Repository,
            selector: "acme/warehouse".into(),
            priority: 0,
            enforcement: EnforcementMode::Observe,
            rules: BTreeMap::from([(
                "SQLCOST001".into(),
                RulePolicy {
                    severity: Some(Severity::Medium),
                    ..RulePolicy::default()
                },
            )]),
            custom_rules: vec![],
            exceptions: vec![],
        });
        let resolved = resolve_policy(
            &document,
            &ResolutionContext {
                organization: "acme",
                repository: "acme/warehouse",
                ..ResolutionContext::default()
            },
        )
        .unwrap();
        assert_eq!(resolved.scope_ids, vec!["org", "repository"]);
        assert_eq!(resolved.enforcement, EnforcementMode::Observe);
        assert_eq!(
            resolved.rules["SQLCOST001"].severity,
            Some(Severity::Medium)
        );
    }
}
