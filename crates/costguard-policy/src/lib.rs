#![forbid(unsafe_code)]

//! Signed policy compilation, verification, and enforcement.
//!
//! Enterprise git-native governance: Ed25519-signed policy documents, scope
//! resolution, declarative custom rules, and exception handling applied to
//! [`Diagnostic`] findings.

use anyhow::{Context, Result};
use base64::Engine;
use chrono::{DateTime, Utc};
use costguard_diagnostics::hex_sha256;
use costguard_diagnostics::{Confidence, Diagnostic, Severity};
use costguard_protocol::{
    AppliedExceptionV1, EnforcementMode, EnforcementOutcome, PolicyProvenanceV1, SignedDocumentV1,
    IDENTITY_SCHEME_SEMANTIC_V1,
};
use ed25519_dalek::{Signer, SigningKey, VerifyingKey};
use globset::Glob;
use rand_core::OsRng;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

const MAX_CUSTOM_RULES: usize = 100;
const MAX_PREDICATE_DEPTH: usize = 12;
const MAX_REGEX_BYTES: usize = 512;
const MAX_MESSAGE_BYTES: usize = 4096;

/// Versioned signed policy document with scopes, rules, and exceptions.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PolicyDocumentV2 {
    pub schema_version: u8,
    pub id: String,
    pub version: String,
    pub organization: String,
    pub issued_at: String,
    pub expires_at: String,
    pub identity_scheme: String,
    #[serde(default)]
    pub permissions: PolicyPermissions,
    #[serde(default)]
    pub scopes: Vec<PolicyScope>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PolicyPermissions {
    #[serde(default)]
    pub allow_repository_baselines: bool,
    #[serde(default)]
    pub allow_inline_suppressions: bool,
    #[serde(default)]
    pub allow_local_severity_overrides: bool,
    #[serde(default)]
    pub allow_cli_overrides: bool,
    #[serde(default)]
    pub allow_local_waivers: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PolicyScope {
    pub id: String,
    pub kind: ScopeKind,
    pub selector: String,
    pub priority: i32,
    #[serde(default)]
    pub enforcement: EnforcementMode,
    #[serde(default)]
    pub rules: BTreeMap<String, RulePolicy>,
    #[serde(default)]
    pub custom_rules: Vec<DeclarativeRule>,
    #[serde(default)]
    pub exceptions: Vec<PolicyException>,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum ScopeKind {
    Organization,
    Team,
    Repository,
    Path,
}

impl ScopeKind {
    fn specificity(self) -> u8 {
        match self {
            Self::Organization => 0,
            Self::Team => 1,
            Self::Repository => 2,
            Self::Path => 3,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RulePolicy {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub severity: Option<Severity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enforcement: Option<EnforcementMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub threshold: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PolicyException {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finding_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule_id: Option<String>,
    pub repository: String,
    pub path: String,
    pub owner: String,
    pub reason: String,
    pub ticket_url: String,
    pub approver: String,
    pub created_at: String,
    pub expires_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeclarativeRule {
    pub id: String,
    pub severity: Severity,
    #[serde(default = "default_confidence")]
    pub confidence: Confidence,
    #[serde(default)]
    pub enforcement: EnforcementMode,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub risk: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggestion: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_multiplier: Option<f64>,
    pub predicate: Predicate,
}

fn default_confidence() -> Confidence {
    Confidence::High
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Predicate {
    All {
        predicates: Vec<Predicate>,
    },
    Any {
        predicates: Vec<Predicate>,
    },
    Not {
        predicate: Box<Predicate>,
    },
    Equals {
        field: String,
        value: String,
    },
    Number {
        field: String,
        comparison: NumberComparison,
        value: f64,
    },
    Glob {
        field: String,
        pattern: String,
    },
    In {
        field: String,
        values: Vec<String>,
    },
    Regex {
        field: String,
        pattern: String,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum NumberComparison {
    Eq,
    Gt,
    Gte,
    Lt,
    Lte,
}

#[derive(Debug, Clone, Default)]
pub struct RuleFactsV1 {
    pub strings: BTreeMap<String, String>,
    pub numbers: BTreeMap<String, f64>,
    pub sets: BTreeMap<String, BTreeSet<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PrivateKeyFileV1 {
    pub key_id: String,
    pub algorithm: String,
    pub created_at: String,
    pub private_key: String,
    pub public_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TrustStoreV1 {
    pub version: u8,
    pub keys: Vec<TrustedKeyV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TrustedKeyV1 {
    pub key_id: String,
    pub algorithm: String,
    pub public_key: String,
    pub valid_from: String,
    pub valid_until: String,
    #[serde(default)]
    pub revoked: bool,
}

#[derive(Debug, Clone, Default)]
pub struct ResolutionContext<'a> {
    pub organization: &'a str,
    pub team: Option<&'a str>,
    pub repository: &'a str,
    pub path: Option<&'a str>,
}

/// Policy resolved for a specific organization, team, repository, and path.
#[derive(Debug, Clone, Serialize)]
pub struct ResolvedPolicy {
    pub document: PolicyDocumentV2,
    pub digest: String,
    pub scope_ids: Vec<String>,
    pub enforcement: EnforcementMode,
    pub rules: BTreeMap<String, RulePolicy>,
    pub custom_rules: Vec<DeclarativeRule>,
    pub exceptions: Vec<PolicyException>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PolicyViolation {
    pub code: String,
    pub message: String,
}

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

pub fn generate_key(key_id: &str, now: DateTime<Utc>) -> Result<PrivateKeyFileV1> {
    require_non_empty("key_id", key_id)?;
    let signing = SigningKey::generate(&mut OsRng);
    let engine = base64::engine::general_purpose::STANDARD;
    Ok(PrivateKeyFileV1 {
        key_id: key_id.into(),
        algorithm: "ed25519".into(),
        created_at: now.to_rfc3339(),
        private_key: engine.encode(signing.to_bytes()),
        public_key: engine.encode(signing.verifying_key().to_bytes()),
    })
}

pub fn sign_policy(policy: &PolicyDocumentV2, key: &PrivateKeyFileV1) -> Result<SignedDocumentV1> {
    validate_policy(policy)?;
    if key.algorithm != "ed25519" {
        anyhow::bail!("unsupported key algorithm '{}'", key.algorithm);
    }
    let private = decode_array::<32>(&key.private_key, "private key")?;
    let signing = SigningKey::from_bytes(&private);
    let canonical = canonical_json(policy)?;
    let signature = signing.sign(canonical.as_bytes());
    let engine = base64::engine::general_purpose::STANDARD;
    Ok(SignedDocumentV1 {
        key_id: key.key_id.clone(),
        algorithm: "ed25519".into(),
        payload: engine.encode(canonical),
        signature: engine.encode(signature.to_bytes()),
    })
}

pub fn verify_policy(
    signed: &SignedDocumentV1,
    trust: &TrustStoreV1,
    now: DateTime<Utc>,
) -> Result<PolicyDocumentV2> {
    if trust.version != 1 {
        anyhow::bail!("unsupported trust store version {}", trust.version);
    }
    if signed.algorithm != "ed25519" {
        anyhow::bail!("unsupported signature algorithm '{}'", signed.algorithm);
    }
    let trusted = trust
        .keys
        .iter()
        .find(|key| key.key_id == signed.key_id)
        .with_context(|| format!("unknown policy signing key '{}'", signed.key_id))?;
    if trusted.revoked {
        anyhow::bail!("policy signing key '{}' is revoked", signed.key_id);
    }
    if trusted.algorithm != "ed25519" {
        anyhow::bail!("unsupported trusted key algorithm '{}'", trusted.algorithm);
    }
    let valid_from = parse_time("trusted key valid_from", &trusted.valid_from)?;
    let valid_until = parse_time("trusted key valid_until", &trusted.valid_until)?;
    if now < valid_from || now > valid_until {
        anyhow::bail!(
            "policy signing key '{}' is outside its validity window",
            signed.key_id
        );
    }
    let engine = base64::engine::general_purpose::STANDARD;
    let payload = engine
        .decode(&signed.payload)
        .context("invalid signed policy payload")?;
    let signature = decode_array::<64>(&signed.signature, "policy signature")?;
    let public = decode_array::<32>(&trusted.public_key, "public key")?;
    let verifying = VerifyingKey::from_bytes(&public).context("invalid Ed25519 public key")?;
    verifying
        .verify_strict(&payload, &ed25519_dalek::Signature::from_bytes(&signature))
        .context("policy signature verification failed")?;
    let value: Value = serde_json::from_slice(&payload).context("invalid signed policy JSON")?;
    if value.get("schema_version").and_then(Value::as_u64) == Some(1) {
        anyhow::bail!(
            "policy schema version 1 is no longer supported; expected schema version {}",
            costguard_protocol::POLICY_SCHEMA_VERSION
        );
    }
    let policy: PolicyDocumentV2 = serde_json::from_value(value)?;
    validate_policy(&policy)?;
    let expires = parse_time("policy.expires_at", &policy.expires_at)?;
    if now > expires {
        anyhow::bail!("policy expired at {}", policy.expires_at);
    }
    if canonical_json(&policy)?.as_bytes() != payload.as_slice() {
        anyhow::bail!("signed policy payload is not canonical JSON");
    }
    Ok(policy)
}

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
    let mut rules = BTreeMap::new();
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
        digest: policy_digest(policy)?,
        scope_ids,
        enforcement,
        rules,
        custom_rules,
        exceptions,
    })
}

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

pub fn evaluate_custom_rule(rule: &DeclarativeRule, facts: &RuleFactsV1) -> Result<bool> {
    validate_custom_rule(rule)?;
    evaluate_predicate(&rule.predicate, facts)
}

pub fn canonical_json<T: Serialize>(value: &T) -> Result<String> {
    let value = serde_json::to_value(value)?;
    serde_json::to_string(&sort_json(value)).context("failed to serialize canonical JSON")
}

pub fn policy_digest(policy: &PolicyDocumentV2) -> Result<String> {
    let canonical = canonical_json(policy)?;
    Ok(format!("sha256:{}", hex_sha256(canonical.as_bytes())))
}

pub fn read_signed_policy(path: &Path) -> Result<SignedDocumentV1> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read signed policy {}", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("invalid signed policy {}", path.display()))
}

pub fn read_trust_store(path: &Path) -> Result<TrustStoreV1> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read trust store {}", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("invalid trust store {}", path.display()))
}

pub fn collect_custom_rule_ids(policy: &PolicyDocumentV2) -> BTreeSet<String> {
    policy
        .scopes
        .iter()
        .flat_map(|scope| scope.custom_rules.iter().map(|rule| rule.id.clone()))
        .collect()
}

fn validate_custom_rule(rule: &DeclarativeRule) -> Result<()> {
    if rule.id.to_ascii_uppercase().starts_with("SQLCOST") || !rule.id.contains('/') {
        anyhow::bail!(
            "custom rule id '{}' must use an organization namespace",
            rule.id
        );
    }
    if rule.message.is_empty() || rule.message.len() > MAX_MESSAGE_BYTES {
        anyhow::bail!(
            "custom rule '{}' message must be 1..={MAX_MESSAGE_BYTES} bytes",
            rule.id
        );
    }
    if rule
        .cost_multiplier
        .is_some_and(|value| !value.is_finite() || value <= 0.0 || value > 1_000.0)
    {
        anyhow::bail!(
            "custom rule '{}' cost_multiplier must be finite and in (0, 1000]",
            rule.id
        );
    }
    validate_predicate(&rule.predicate, 0)
}

fn validate_predicate(predicate: &Predicate, depth: usize) -> Result<()> {
    if depth > MAX_PREDICATE_DEPTH {
        anyhow::bail!("custom rule predicate exceeds maximum depth {MAX_PREDICATE_DEPTH}");
    }
    match predicate {
        Predicate::All { predicates } | Predicate::Any { predicates } => {
            if predicates.is_empty() {
                anyhow::bail!("all/any predicates cannot be empty");
            }
            for child in predicates {
                validate_predicate(child, depth + 1)?;
            }
        }
        Predicate::Not { predicate } => validate_predicate(predicate, depth + 1)?,
        Predicate::Equals { field, .. }
        | Predicate::Glob { field, .. }
        | Predicate::Regex { field, .. } => validate_field(field, FieldKind::String)?,
        Predicate::In { field, .. } => validate_membership_field(field)?,
        Predicate::Number { field, value, .. } => {
            validate_field(field, FieldKind::Number)?;
            if !value.is_finite() {
                anyhow::bail!("numeric predicate value must be finite");
            }
        }
    }
    if let Predicate::Glob { pattern, .. } = predicate {
        Glob::new(pattern).context("invalid declarative-rule glob")?;
    }
    if let Predicate::Regex { pattern, .. } = predicate {
        if pattern.len() > MAX_REGEX_BYTES {
            anyhow::bail!("declarative-rule regex exceeds {MAX_REGEX_BYTES} bytes");
        }
        regex::Regex::new(pattern).context("invalid declarative-rule regex")?;
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum FieldKind {
    String,
    Number,
    Set,
}

fn validate_field(field: &str, expected: FieldKind) -> Result<()> {
    require_non_empty("predicate field", field)?;
    let actual = field_kind(field)
        .with_context(|| format!("unsupported declarative-rule field '{field}'"))?;
    if std::mem::discriminant(&actual) != std::mem::discriminant(&expected) {
        anyhow::bail!("operator is incompatible with declarative-rule field '{field}'");
    }
    Ok(())
}

fn validate_membership_field(field: &str) -> Result<()> {
    require_non_empty("predicate field", field)?;
    match field_kind(field) {
        Some(FieldKind::String | FieldKind::Set) => Ok(()),
        Some(FieldKind::Number) => {
            anyhow::bail!("membership operator is incompatible with numeric field '{field}'")
        }
        None => anyhow::bail!("unsupported declarative-rule field '{field}'"),
    }
}

fn field_kind(field: &str) -> Option<FieldKind> {
    if matches!(
        field,
        "platform"
            | "path"
            | "file.kind"
            | "metadata.state"
            | "dbt.materialized"
            | "dbt.model"
            | "dbt.unique_key"
            | "dbt.incremental_strategy"
            | "dbt.partition_by"
            | "dbt.cluster_by"
            | "dbt.on_schema_change"
            | "sql.parsed"
    ) {
        Some(FieldKind::String)
    } else if matches!(
        field,
        "dbt.tags" | "sql.function_classes" | "sql.join_kinds" | "lineage.refs" | "lineage.sources"
    ) {
        Some(FieldKind::Set)
    } else if matches!(
        field,
        "dbt.ref_count"
            | "dbt.source_count"
            | "sql.join_count"
            | "sql.equality_join_count"
            | "sql.unbounded_join_count"
            | "sql.window_count"
            | "sql.unpartitioned_window_count"
            | "sql.cte_count"
            | "sql.select_star_count"
            | "sql.json_extraction_count"
            | "sql.regex_count"
            | "sql.normalization_count"
            | "sql.non_sargable_count"
            | "sql.correlated_subquery_count"
            | "sql.cross_join_count"
    ) {
        Some(FieldKind::Number)
    } else {
        None
    }
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

fn scope_matches(scope: &PolicyScope, context: &ResolutionContext<'_>) -> Result<bool> {
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

fn scopes_conflict(left: &PolicyScope, right: &PolicyScope) -> bool {
    left.enforcement != right.enforcement
        || left
            .rules
            .iter()
            .any(|(id, rule)| right.rules.get(id).is_some_and(|other| other != rule))
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

fn evaluate_predicate(predicate: &Predicate, facts: &RuleFactsV1) -> Result<bool> {
    Ok(match predicate {
        Predicate::All { predicates } => predicates
            .iter()
            .map(|item| evaluate_predicate(item, facts))
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .all(|matched| matched),
        Predicate::Any { predicates } => predicates
            .iter()
            .map(|item| evaluate_predicate(item, facts))
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .any(|matched| matched),
        Predicate::Not { predicate } => !evaluate_predicate(predicate, facts)?,
        Predicate::Equals { field, value } => facts.strings.get(field) == Some(value),
        Predicate::Number {
            field,
            comparison,
            value,
        } => facts
            .numbers
            .get(field)
            .is_some_and(|actual| match comparison {
                NumberComparison::Eq => actual == value,
                NumberComparison::Gt => actual > value,
                NumberComparison::Gte => actual >= value,
                NumberComparison::Lt => actual < value,
                NumberComparison::Lte => actual <= value,
            }),
        Predicate::Glob { field, pattern } => facts.strings.get(field).is_some_and(|value| {
            Glob::new(pattern)
                .map(|glob| glob.compile_matcher().is_match(value))
                .unwrap_or(false)
        }),
        Predicate::In { field, values } => {
            facts
                .strings
                .get(field)
                .is_some_and(|value| values.contains(value))
                || facts
                    .sets
                    .get(field)
                    .is_some_and(|set| values.iter().any(|value| set.contains(value)))
        }
        Predicate::Regex { field, pattern } => facts.strings.get(field).is_some_and(|value| {
            regex::Regex::new(pattern)
                .map(|regex| regex.is_match(value))
                .unwrap_or(false)
        }),
    })
}

fn parse_time(name: &str, value: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .map(|time| time.with_timezone(&Utc))
        .with_context(|| format!("{name} must be RFC3339"))
}

fn require_non_empty(name: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() {
        anyhow::bail!("{name} cannot be empty");
    }
    Ok(())
}

fn decode_array<const N: usize>(value: &str, label: &str) -> Result<[u8; N]> {
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(value)
        .with_context(|| format!("invalid base64 {label}"))?;
    decoded
        .try_into()
        .map_err(|_| anyhow::anyhow!("{label} must be {N} bytes"))
}

fn sort_json(value: Value) -> Value {
    match value {
        Value::Object(object) => {
            let sorted = object
                .into_iter()
                .map(|(key, value)| (key, sort_json(value)))
                .collect::<BTreeMap<_, _>>();
            Value::Object(sorted.into_iter().collect::<Map<_, _>>())
        }
        Value::Array(values) => Value::Array(values.into_iter().map(sort_json).collect()),
        other => other,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
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
    fn signed_policy_round_trip_and_tamper_rejection() {
        let now = Utc::now();
        let policy = policy(now);
        let key = generate_key("root-2026", now).unwrap();
        let trust = TrustStoreV1 {
            version: 1,
            keys: vec![TrustedKeyV1 {
                key_id: key.key_id.clone(),
                algorithm: "ed25519".into(),
                public_key: key.public_key.clone(),
                valid_from: (now - chrono::Duration::minutes(1)).to_rfc3339(),
                valid_until: (now + chrono::Duration::days(365)).to_rfc3339(),
                revoked: false,
            }],
        };
        let signed = sign_policy(&policy, &key).unwrap();
        assert_eq!(verify_policy(&signed, &trust, now).unwrap().id, policy.id);
        let mut tampered = signed;
        tampered.payload.push('A');
        assert!(verify_policy(&tampered, &trust, now).is_err());
    }

    #[test]
    fn revoked_and_expired_keys_fail_closed() {
        let now = Utc::now();
        let key = generate_key("root", now).unwrap();
        let signed = sign_policy(&policy(now), &key).unwrap();
        let mut trust = TrustStoreV1 {
            version: 1,
            keys: vec![TrustedKeyV1 {
                key_id: key.key_id,
                algorithm: "ed25519".into(),
                public_key: key.public_key,
                valid_from: (now - chrono::Duration::days(2)).to_rfc3339(),
                valid_until: (now + chrono::Duration::days(2)).to_rfc3339(),
                revoked: true,
            }],
        };
        assert!(verify_policy(&signed, &trust, now).is_err());
        trust.keys[0].revoked = false;
        trust.keys[0].valid_until = (now - chrono::Duration::days(1)).to_rfc3339();
        assert!(verify_policy(&signed, &trust, now).is_err());
    }

    #[test]
    fn unknown_policy_key_fails_closed() {
        let now = Utc::now();
        let key = generate_key("unknown-root", now).unwrap();
        let signed = sign_policy(&policy(now), &key).unwrap();
        let trust = TrustStoreV1 {
            version: 1,
            keys: vec![],
        };
        let error = verify_policy(&signed, &trust, now).unwrap_err().to_string();
        assert!(error.contains("unknown policy signing key"), "{error}");
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

    #[test]
    fn declarative_rules_are_bounded_and_evaluated() {
        let rule = DeclarativeRule {
            id: "acme/no-large-joins".into(),
            severity: Severity::High,
            confidence: Confidence::High,
            enforcement: EnforcementMode::Block,
            message: "too many joins".into(),
            risk: None,
            suggestion: None,
            cost_multiplier: None,
            predicate: Predicate::All {
                predicates: vec![
                    Predicate::Glob {
                        field: "path".into(),
                        pattern: "models/marts/**".into(),
                    },
                    Predicate::Number {
                        field: "sql.join_count".into(),
                        comparison: NumberComparison::Gte,
                        value: 4.0,
                    },
                ],
            },
        };
        let mut facts = RuleFactsV1::default();
        facts
            .strings
            .insert("path".into(), "models/marts/fct.sql".into());
        facts.numbers.insert("sql.join_count".into(), 5.0);
        assert!(evaluate_custom_rule(&rule, &facts).unwrap());
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
