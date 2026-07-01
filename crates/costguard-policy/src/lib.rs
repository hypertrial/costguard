#![forbid(unsafe_code)]

//! Signed policy compilation, verification, and enforcement.
//!
//! Enterprise git-native governance: Ed25519-signed policy documents, scope
//! resolution, declarative custom rules, and exception handling applied to
//! [`costguard_diagnostics::Diagnostic`] findings.

mod compile;
mod governance;
mod resolve;
mod rules;
mod sign;
mod types;

use anyhow::{Context, Result};
use costguard_diagnostics::hex_sha256;
use costguard_protocol::SignedDocumentV1;
use serde::Serialize;
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::path::Path;

pub use compile::{compile_toml, validate_policy};
pub use governance::apply_governance;
pub use resolve::resolve_policy;
pub use rules::{collect_custom_rule_ids, evaluate_custom_rule};
pub use sign::{generate_key, sign_policy, verify_policy};
pub use types::*;

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
