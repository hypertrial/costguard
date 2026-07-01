use costguard_diagnostics::{Confidence, Severity};
use costguard_protocol::EnforcementMode;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

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
    pub(crate) fn specificity(self) -> u8 {
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

pub(crate) fn default_confidence() -> Confidence {
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
    pub sets: BTreeMap<String, std::collections::BTreeSet<String>>,
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
