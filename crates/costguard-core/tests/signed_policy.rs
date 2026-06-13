use chrono::Utc;
use costguard_core::{scan, ScanConfig, SignedPolicyConfig};
use costguard_diagnostics::Severity;
use costguard_policy::{
    generate_key, sign_policy, DeclarativeRule, PolicyDocumentV1, PolicyPermissions, PolicyScope,
    Predicate, ScopeKind, TrustStoreV1, TrustedKeyV1,
};
use costguard_protocol::{EnforcementMode, EnforcementOutcome};
use std::collections::BTreeMap;
use std::fs;
use tempfile::TempDir;

fn managed_scan(enforcement: EnforcementMode) -> costguard_core::ScanResult {
    let temp = TempDir::new().unwrap();
    let model_dir = temp.path().join("models/marts");
    fs::create_dir_all(&model_dir).unwrap();
    fs::write(
        model_dir.join("orders.sql"),
        "-- costguard: disable-file=SQLCOST001\nselect * from raw.orders\n",
    )
    .unwrap();

    let now = Utc::now();
    let policy = PolicyDocumentV1 {
        schema_version: 1,
        id: "org-default".into(),
        version: "2026.06".into(),
        organization: "acme".into(),
        issued_at: (now - chrono::Duration::minutes(1)).to_rfc3339(),
        expires_at: (now + chrono::Duration::days(30)).to_rfc3339(),
        permissions: PolicyPermissions::default(),
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
    };
    let key = generate_key("root-2026", now).unwrap();
    let signed = sign_policy(&policy, &key).unwrap();
    let trust = TrustStoreV1 {
        version: 1,
        keys: vec![TrustedKeyV1 {
            key_id: key.key_id,
            algorithm: key.algorithm,
            public_key: key.public_key,
            valid_from: (now - chrono::Duration::days(1)).to_rfc3339(),
            valid_until: (now + chrono::Duration::days(365)).to_rfc3339(),
            revoked: false,
        }],
    };
    let bundle = temp.path().join("policy.json");
    let trust_store = temp.path().join("trust.json");
    fs::write(&bundle, serde_json::to_vec_pretty(&signed).unwrap()).unwrap();
    fs::write(&trust_store, serde_json::to_vec_pretty(&trust).unwrap()).unwrap();

    scan(&ScanConfig {
        root: temp.path().to_path_buf(),
        paths: vec![temp.path().join("models")],
        fail_on: Some(Severity::Medium),
        signed_policy: SignedPolicyConfig {
            bundle_path: Some(bundle),
            trust_store_path: Some(trust_store),
            organization: Some("acme".into()),
            repository: Some("acme/warehouse".into()),
            required: true,
            ..SignedPolicyConfig::default()
        },
        ..ScanConfig::default()
    })
    .unwrap()
}

#[test]
fn managed_policy_disables_inline_suppression_and_runs_custom_rules() {
    let result = managed_scan(EnforcementMode::Block);
    assert_ne!(result.policy.digest, "local-unmanaged");
    assert!(result
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.rule_id == "SQLCOST001"));
    assert!(result
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.rule_id == "acme/mart-select"));
    assert!(result.diagnostics.iter().all(|diagnostic| {
        diagnostic.governance.policy.is_some()
            && !diagnostic.governance.finding_id.is_empty()
            && !diagnostic.governance.evidence_key.is_empty()
    }));
    assert!(result.should_fail(Some(Severity::Medium), None, None, None));
}

#[test]
fn managed_observe_findings_do_not_fail_severity_gate() {
    let result = managed_scan(EnforcementMode::Observe);
    assert!(result.diagnostics.iter().any(|diagnostic| {
        diagnostic.rule_id == "SQLCOST001"
            && diagnostic.governance.enforcement == EnforcementOutcome::Observed
    }));
    assert!(!result.should_fail(Some(Severity::Medium), None, None, None));
}
