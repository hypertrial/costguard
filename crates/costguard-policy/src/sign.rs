use crate::canonical_json;
use crate::compile::validate_policy;
use crate::types::{PolicyDocumentV2, PrivateKeyFileV1, TrustStoreV1};
use anyhow::{Context, Result};
use base64::Engine;
use chrono::{DateTime, Utc};
use costguard_protocol::SignedDocumentV1;
use ed25519_dalek::{Signer, SigningKey, VerifyingKey};
use rand_core::OsRng;

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
    let value: serde_json::Value =
        serde_json::from_slice(&payload).context("invalid signed policy JSON")?;
    if value
        .get("schema_version")
        .and_then(serde_json::Value::as_u64)
        == Some(1)
    {
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

pub(crate) fn parse_time(name: &str, value: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .map(|time| time.with_timezone(&Utc))
        .with_context(|| format!("{name} must be RFC3339"))
}

pub(crate) fn require_non_empty(name: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() {
        anyhow::bail!("{name} cannot be empty");
    }
    Ok(())
}

pub(crate) fn decode_array<const N: usize>(value: &str, label: &str) -> Result<[u8; N]> {
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(value)
        .with_context(|| format!("invalid base64 {label}"))?;
    decoded
        .try_into()
        .map_err(|_| anyhow::anyhow!("{label} must be {N} bytes"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{PolicyPermissions, PolicyScope, ScopeKind, TrustedKeyV1};
    use chrono::Utc;
    use costguard_protocol::{EnforcementMode, IDENTITY_SCHEME_SEMANTIC_V1};
    use std::collections::BTreeMap;

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
}
