#![forbid(unsafe_code)]

use schemars::{schema::RootSchema, JsonSchema};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const POLICY_SCHEMA_VERSION: u8 = 1;
pub const BASELINE_SCHEMA_VERSION: u8 = 2;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EnforcementMode {
    #[default]
    Observe,
    Warn,
    Block,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EnforcementOutcome {
    #[default]
    Observed,
    Warned,
    Blocked,
    Excepted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PolicyProvenanceV1 {
    pub digest: String,
    pub version: String,
    pub scope: String,
}

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
pub struct FindingV1 {
    pub finding_id: String,
    pub evidence_key: String,
    pub rule_id: String,
    pub severity: String,
    pub confidence: String,
    pub path: String,
    pub line: usize,
    pub column: usize,
    pub message: String,
    pub enforcement: EnforcementOutcome,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<PolicyProvenanceV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exception: Option<AppliedExceptionV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<Value>,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SignedDocumentV1 {
    pub key_id: String,
    pub algorithm: String,
    pub payload: String,
    pub signature: String,
}

pub fn schema_for<T: JsonSchema>() -> RootSchema {
    schemars::schema_for!(T)
}
