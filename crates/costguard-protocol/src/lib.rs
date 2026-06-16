#![forbid(unsafe_code)]

//! Shared JSON schema types.
//!
//! Protocol types for signed documents, baselines, cost observation bundles,
//! and enforcement outcomes. All public structs derive [`JsonSchema`] for
//! schema generation.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const POLICY_SCHEMA_VERSION: u8 = 2;
pub const BASELINE_SCHEMA_VERSION: u8 = 3;
pub const IDENTITY_SCHEME_SEMANTIC_V1: &str = "semantic-v1";

/// Canonical built-in rule IDs (SQLCOST001..SQLCOST045).
pub const BUILTIN_RULE_IDS: [&str; 45] = [
    "SQLCOST001",
    "SQLCOST002",
    "SQLCOST003",
    "SQLCOST004",
    "SQLCOST005",
    "SQLCOST006",
    "SQLCOST007",
    "SQLCOST008",
    "SQLCOST009",
    "SQLCOST010",
    "SQLCOST011",
    "SQLCOST012",
    "SQLCOST013",
    "SQLCOST014",
    "SQLCOST015",
    "SQLCOST016",
    "SQLCOST017",
    "SQLCOST018",
    "SQLCOST019",
    "SQLCOST020",
    "SQLCOST021",
    "SQLCOST022",
    "SQLCOST023",
    "SQLCOST024",
    "SQLCOST025",
    "SQLCOST026",
    "SQLCOST027",
    "SQLCOST028",
    "SQLCOST029",
    "SQLCOST030",
    "SQLCOST031",
    "SQLCOST032",
    "SQLCOST033",
    "SQLCOST034",
    "SQLCOST035",
    "SQLCOST036",
    "SQLCOST037",
    "SQLCOST038",
    "SQLCOST039",
    "SQLCOST040",
    "SQLCOST041",
    "SQLCOST042",
    "SQLCOST043",
    "SQLCOST044",
    "SQLCOST045",
];

/// Returns true when `rule_id` is a known built-in SQLCOST rule.
pub fn is_builtin_rule_id(rule_id: &str) -> bool {
    let upper = rule_id.to_ascii_uppercase();
    BUILTIN_RULE_IDS.contains(&upper.as_str())
}

/// Policy enforcement mode: observe, warn, or block findings.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EnforcementMode {
    #[default]
    Observe,
    Warn,
    Block,
}

/// Resolved enforcement outcome after policy and exception evaluation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EnforcementOutcome {
    #[default]
    Observed,
    Warned,
    Blocked,
    Excepted,
}

/// Provenance linking a finding to the signed policy that governed it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PolicyProvenanceV1 {
    pub digest: String,
    pub version: String,
    pub scope: String,
}

/// An approved exception that suppresses enforcement for a matching finding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AppliedExceptionV1 {
    pub id: String,
    pub owner: String,
    pub reason: String,
    pub ticket_url: String,
    pub approver: String,
    pub expires_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CostObservationBundleV1 {
    pub schema_version: u8,
    pub organization: String,
    pub repository: String,
    pub currency: String,
    pub generated_at: String,
    pub provenance: String,
    pub observations: Vec<ModelCostObservationV1>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ModelCostObservationV1 {
    pub model_id: String,
    pub window_start: String,
    pub window_end: String,
    pub executions: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes_processed: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compute_seconds: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credits: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
}

/// Ed25519-signed document envelope (policy, baseline, or cost bundle payload).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SignedDocumentV1 {
    pub key_id: String,
    pub algorithm: String,
    pub payload: String,
    pub signature: String,
}
