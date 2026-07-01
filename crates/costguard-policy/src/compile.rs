use crate::rules::{collect_custom_rule_ids, validate_custom_rule};
use crate::sign::{parse_time, require_non_empty};
use crate::types::{PolicyDocumentV2, PolicyException, ScopeKind};
use anyhow::{Context, Result};
use costguard_protocol::IDENTITY_SCHEME_SEMANTIC_V1;
use globset::Glob;
use std::collections::BTreeSet;

pub(crate) const MAX_CUSTOM_RULES: usize = 100;

pub fn compile_toml(text: &str) -> Result<PolicyDocumentV2> {
    let policy: PolicyDocumentV2 = toml::from_str(text).context("invalid policy TOML")?;
    validate_policy(&policy)?;
    Ok(policy)
}

pub fn validate_policy(policy: &PolicyDocumentV2) -> Result<()> {
    if policy.schema_version != costguard_protocol::POLICY_SCHEMA_VERSION {
        anyhow::bail!(
            "unsupported policy schema version {}; expected {}",
            policy.schema_version,
            costguard_protocol::POLICY_SCHEMA_VERSION
        );
    }
    if policy.identity_scheme != IDENTITY_SCHEME_SEMANTIC_V1 {
        anyhow::bail!(
            "unsupported identity scheme '{}'; expected '{IDENTITY_SCHEME_SEMANTIC_V1}'",
            policy.identity_scheme
        );
    }
    require_non_empty("policy.id", &policy.id)?;
    require_non_empty("policy.version", &policy.version)?;
    require_non_empty("policy.organization", &policy.organization)?;
    let issued = parse_time("policy.issued_at", &policy.issued_at)?;
    let expires = parse_time("policy.expires_at", &policy.expires_at)?;
    if expires <= issued {
        anyhow::bail!("policy.expires_at must be after issued_at");
    }

    let mut scope_ids = BTreeSet::new();
    let custom_rule_ids = collect_custom_rule_ids(policy);
    let mut seen_custom_rule_ids = BTreeSet::new();
    let custom_rule_count = policy
        .scopes
        .iter()
        .map(|scope| scope.custom_rules.len())
        .sum::<usize>();
    if custom_rule_count > MAX_CUSTOM_RULES {
        anyhow::bail!("policy has {custom_rule_count} custom rules; maximum is {MAX_CUSTOM_RULES}");
    }
    for scope in &policy.scopes {
        require_non_empty("scope.id", &scope.id)?;
        require_non_empty("scope.selector", &scope.selector)?;
        if !scope_ids.insert(scope.id.clone()) {
            anyhow::bail!("duplicate policy scope id '{}'", scope.id);
        }
        if matches!(scope.kind, ScopeKind::Path) {
            Glob::new(&scope.selector)
                .with_context(|| format!("invalid path scope '{}'", scope.selector))?;
        }
        for (rule_id, rule) in &scope.rules {
            validate_builtin_rule_id(rule_id)?;
            if rule.threshold == Some(0) {
                anyhow::bail!("rule {rule_id} threshold must be greater than zero");
            }
        }
        for rule in &scope.custom_rules {
            validate_custom_rule(rule)?;
            if !seen_custom_rule_ids.insert(rule.id.clone()) {
                anyhow::bail!("duplicate custom rule id '{}'", rule.id);
            }
        }
        for exception in &scope.exceptions {
            validate_exception(exception, &custom_rule_ids)?;
        }
    }
    Ok(())
}

fn validate_exception(
    exception: &PolicyException,
    custom_rule_ids: &BTreeSet<String>,
) -> Result<()> {
    for (name, value) in [
        ("exception.id", exception.id.as_str()),
        ("exception.repository", exception.repository.as_str()),
        ("exception.path", exception.path.as_str()),
        ("exception.owner", exception.owner.as_str()),
        ("exception.reason", exception.reason.as_str()),
        ("exception.ticket_url", exception.ticket_url.as_str()),
        ("exception.approver", exception.approver.as_str()),
    ] {
        require_non_empty(name, value)?;
    }
    if exception.finding_id.is_none() && exception.rule_id.is_none() {
        anyhow::bail!(
            "exception '{}' must specify finding_id or rule_id",
            exception.id
        );
    }
    if let Some(rule_id) = &exception.rule_id {
        if !costguard_protocol::is_builtin_rule_id(rule_id) && !custom_rule_ids.contains(rule_id) {
            anyhow::bail!(
                "exception '{}' references unknown rule id '{rule_id}'",
                exception.id
            );
        }
    }
    Glob::new(&exception.repository).context("invalid exception repository glob")?;
    Glob::new(&exception.path).context("invalid exception path glob")?;
    let created = parse_time("exception.created_at", &exception.created_at)?;
    let expires = parse_time("exception.expires_at", &exception.expires_at)?;
    if expires <= created {
        anyhow::bail!(
            "exception '{}' expires_at must be after created_at",
            exception.id
        );
    }
    Ok(())
}

fn validate_builtin_rule_id(rule_id: &str) -> Result<()> {
    if !costguard_protocol::is_builtin_rule_id(rule_id) {
        anyhow::bail!("unknown built-in rule id '{rule_id}'");
    }
    Ok(())
}
