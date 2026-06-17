//! Shared helpers for `costguard-core` integration tests.
#![allow(dead_code)] // ponytail: each integration test binary includes this module independently; not all helpers used in every binary.

use chrono::{DateTime, Utc};
use costguard_diagnostics::Severity;
use costguard_policy::{
    generate_key, sign_policy, DeclarativeRule, PolicyDocumentV2, PolicyPermissions, PolicyScope,
    Predicate, ScopeKind, TrustStoreV1, TrustedKeyV1,
};
use costguard_protocol::{EnforcementMode, SignedDocumentV1, IDENTITY_SCHEME_SEMANTIC_V1};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

pub fn fixture(name: &str) -> PathBuf {
    repo_root().join("tests/fixtures/corpus").join(name)
}

pub fn run_git(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .expect("run git");
    assert!(
        output.status.success(),
        "git {} failed:\n{}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
}

pub fn copy_dir_all(source: &Path, destination: &Path) -> std::io::Result<()> {
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let target = destination.join(entry.file_name());
        if file_type.is_dir() {
            copy_dir_all(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

pub struct SignedPolicyFiles {
    pub bundle_path: PathBuf,
    pub trust_store_path: PathBuf,
}

pub fn write_managed_scan_model(root: &Path) {
    let model_dir = root.join("models/marts");
    fs::create_dir_all(&model_dir).expect("create models/marts");
    fs::write(
        model_dir.join("orders.sql"),
        "-- costguard: disable-file=SQLCOST001\nselect * from raw.orders\n",
    )
    .expect("write orders model");
}

pub fn write_managed_policy_bundle(
    root: &Path,
    enforcement: EnforcementMode,
    permissions: PolicyPermissions,
    mutate_policy: impl FnOnce(&mut PolicyDocumentV2),
    mutate_signed: impl FnOnce(&mut SignedDocumentV1, &mut TrustStoreV1),
) -> SignedPolicyFiles {
    let now = Utc::now();
    let mut policy = managed_policy_document(now, enforcement, permissions);
    mutate_policy(&mut policy);
    let key = generate_key("root-2026", now).expect("generate key");
    let mut signed = sign_policy(&policy, &key).expect("sign policy");
    let mut trust = managed_trust_store(&key, now);
    mutate_signed(&mut signed, &mut trust);
    let bundle_path = root.join("policy.json");
    let trust_store_path = root.join("trust.json");
    fs::write(
        &bundle_path,
        serde_json::to_vec_pretty(&signed).expect("serialize policy"),
    )
    .expect("write policy bundle");
    fs::write(
        &trust_store_path,
        serde_json::to_vec_pretty(&trust).expect("serialize trust store"),
    )
    .expect("write trust store");
    SignedPolicyFiles {
        bundle_path,
        trust_store_path,
    }
}

fn managed_policy_document(
    now: DateTime<Utc>,
    enforcement: EnforcementMode,
    permissions: PolicyPermissions,
) -> PolicyDocumentV2 {
    PolicyDocumentV2 {
        schema_version: costguard_protocol::POLICY_SCHEMA_VERSION,
        id: "org-default".into(),
        version: "2026.06".into(),
        organization: "acme".into(),
        issued_at: (now - chrono::Duration::minutes(1)).to_rfc3339(),
        expires_at: (now + chrono::Duration::days(30)).to_rfc3339(),
        identity_scheme: IDENTITY_SCHEME_SEMANTIC_V1.into(),
        permissions,
        scopes: vec![PolicyScope {
            id: "org-default".into(),
            kind: ScopeKind::Organization,
            selector: "acme".into(),
            priority: 0,
            enforcement,
            rules: BTreeMap::new(),
            custom_rules: vec![DeclarativeRule {
                id: "acme/mart-select".into(),
                severity: Severity::High,
                confidence: costguard_diagnostics::Confidence::High,
                enforcement: EnforcementMode::Block,
                message: "Mart SQL requires policy review.".into(),
                risk: None,
                suggestion: None,
                cost_multiplier: Some(1.25),
                predicate: Predicate::Glob {
                    field: "path".into(),
                    pattern: "models/marts/**".into(),
                },
            }],
            exceptions: vec![],
        }],
    }
}

fn managed_trust_store(
    key: &costguard_policy::PrivateKeyFileV1,
    now: DateTime<Utc>,
) -> TrustStoreV1 {
    TrustStoreV1 {
        version: 1,
        keys: vec![TrustedKeyV1 {
            key_id: key.key_id.clone(),
            algorithm: key.algorithm.clone(),
            public_key: key.public_key.clone(),
            valid_from: (now - chrono::Duration::days(1)).to_rfc3339(),
            valid_until: (now + chrono::Duration::days(365)).to_rfc3339(),
            revoked: false,
        }],
    }
}
