use chrono::Utc;
use costguard_core::{scan, FindingBaseline, ScanConfig, SignedPolicyConfig};
use costguard_diagnostics::Severity;
use costguard_policy::{
    generate_key, sign_policy, DeclarativeRule, PolicyDocumentV2, PolicyException,
    PolicyPermissions, PolicyScope, Predicate, RulePolicy, ScopeKind, TrustStoreV1, TrustedKeyV1,
};
use costguard_protocol::{
    EnforcementMode, EnforcementOutcome, SignedDocumentV1, IDENTITY_SCHEME_SEMANTIC_V1,
};
use std::collections::BTreeMap;
use std::fs;
use tempfile::TempDir;

fn managed_scan_custom(
    enforcement: EnforcementMode,
    permissions: PolicyPermissions,
    mutate_policy: impl FnOnce(&mut PolicyDocumentV2),
    mutate_signed: impl FnOnce(&mut SignedDocumentV1, &mut TrustStoreV1),
    configure: impl FnOnce(&TempDir, &mut ScanConfig),
) -> anyhow::Result<costguard_core::ScanResult> {
    let temp = TempDir::new().unwrap();
    let model_dir = temp.path().join("models/marts");
    fs::create_dir_all(&model_dir).unwrap();
    fs::write(
        model_dir.join("orders.sql"),
        "-- costguard: disable-file=SQLCOST001\nselect * from raw.orders\n",
    )
    .unwrap();

    let now = Utc::now();
    let mut policy = PolicyDocumentV2 {
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
    };
    mutate_policy(&mut policy);
    let key = generate_key("root-2026", now).unwrap();
    let mut signed = sign_policy(&policy, &key).unwrap();
    let mut trust = TrustStoreV1 {
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
    mutate_signed(&mut signed, &mut trust);
    let bundle = temp.path().join("policy.json");
    let trust_store = temp.path().join("trust.json");
    fs::write(&bundle, serde_json::to_vec_pretty(&signed).unwrap()).unwrap();
    fs::write(&trust_store, serde_json::to_vec_pretty(&trust).unwrap()).unwrap();

    let mut config = ScanConfig {
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
    };
    configure(&temp, &mut config);
    scan(&config)
}

fn managed_scan_with(
    enforcement: EnforcementMode,
    permissions: PolicyPermissions,
    configure: impl FnOnce(&TempDir, &mut ScanConfig),
) -> anyhow::Result<costguard_core::ScanResult> {
    managed_scan_custom(
        enforcement,
        permissions,
        |_policy| {},
        |_signed, _trust| {},
        configure,
    )
}

fn managed_scan(enforcement: EnforcementMode) -> costguard_core::ScanResult {
    managed_scan_with(
        enforcement,
        PolicyPermissions::default(),
        |_temp, _config| {},
    )
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

#[test]
fn managed_policy_rejects_forbidden_local_overrides() {
    let error = managed_scan_with(
        EnforcementMode::Block,
        PolicyPermissions::default(),
        |_temp, config| {
            config.rule_overrides.insert(
                "SQLCOST001".into(),
                costguard_rules::RuleOverride {
                    enabled: Some(false),
                    ..costguard_rules::RuleOverride::default()
                },
            );
        },
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("forbids local rule overrides"), "{error}");
}

#[test]
fn managed_policy_rejects_baseline_digest_mismatch() {
    let error = managed_scan_with(
        EnforcementMode::Block,
        PolicyPermissions {
            allow_repository_baselines: true,
            ..PolicyPermissions::default()
        },
        |temp, config| {
            let path = temp.path().join("baseline.json");
            let baseline = FindingBaseline {
                version: costguard_protocol::BASELINE_SCHEMA_VERSION,
                identity_scheme: IDENTITY_SCHEME_SEMANTIC_V1.into(),
                platform: "generic".into(),
                generated_at: Utc::now().to_rfc3339(),
                policy_digest: Some("sha256:not-the-active-policy".into()),
                findings: vec![],
            };
            fs::write(&path, serde_json::to_vec_pretty(&baseline).unwrap()).unwrap();
            config.baseline_path = Some(path);
        },
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("does not match active policy"), "{error}");
}

#[test]
fn managed_scan_rejects_tampered_policy_bundle() {
    let error = managed_scan_custom(
        EnforcementMode::Block,
        PolicyPermissions::default(),
        |_policy| {},
        |signed, _trust| signed.payload.push('A'),
        |_temp, _config| {},
    )
    .unwrap_err()
    .to_string();
    assert!(
        error.contains("policy") || error.contains("signature"),
        "{error}"
    );
}

#[test]
fn managed_scan_rejects_revoked_expired_and_unknown_policy_keys() {
    type TrustMutation = fn(&mut TrustStoreV1);
    let cases: [(&str, TrustMutation); 3] = [
        ("revoked", |trust| trust.keys[0].revoked = true),
        ("validity window", |trust| {
            trust.keys[0].valid_until = "2000-01-01T00:00:00Z".into()
        }),
        ("unknown", |trust| trust.keys.clear()),
    ];
    for (expected, mutate_trust) in cases {
        let error = managed_scan_custom(
            EnforcementMode::Block,
            PolicyPermissions::default(),
            |_policy| {},
            |_signed, trust| mutate_trust(trust),
            |_temp, _config| {},
        )
        .unwrap_err()
        .to_string();
        assert!(
            error.contains(expected),
            "expected {expected} error, got: {error}"
        );
    }
}

#[test]
fn managed_scan_keeps_findings_when_exception_is_expired() {
    let now = Utc::now();
    let result = managed_scan_custom(
        EnforcementMode::Block,
        PolicyPermissions::default(),
        |policy| {
            policy.scopes[0].exceptions.push(PolicyException {
                id: "expired-select-star".into(),
                finding_id: None,
                rule_id: Some("SQLCOST001".into()),
                repository: "acme/warehouse".into(),
                path: "models/**".into(),
                owner: "data-platform".into(),
                reason: "migration window ended".into(),
                ticket_url: "https://tickets.example/expired".into(),
                approver: "security".into(),
                created_at: (now - chrono::Duration::days(2)).to_rfc3339(),
                expires_at: (now - chrono::Duration::days(1)).to_rfc3339(),
            });
        },
        |_signed, _trust| {},
        |_temp, _config| {},
    )
    .unwrap();
    assert!(result
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.rule_id == "SQLCOST001"));
    assert!(result
        .analysis
        .violations
        .iter()
        .any(|violation| violation.code == "expired_exception"));
    assert!(!result.analysis.passed);
}

#[test]
fn path_scoped_rule_overrides_apply_per_file_not_globally() {
    let temp = TempDir::new().unwrap();
    let marts_dir = temp.path().join("models/marts");
    let intermediate_dir = temp.path().join("models/intermediate");
    fs::create_dir_all(&marts_dir).unwrap();
    fs::create_dir_all(&intermediate_dir).unwrap();
    fs::write(marts_dir.join("orders.sql"), "select * from raw.orders\n").unwrap();
    fs::write(
        intermediate_dir.join("customers.sql"),
        "select * from raw.customers\n",
    )
    .unwrap();

    let now = Utc::now();
    let mut policy = PolicyDocumentV2 {
        schema_version: costguard_protocol::POLICY_SCHEMA_VERSION,
        id: "org-default".into(),
        version: "2026.06".into(),
        organization: "acme".into(),
        issued_at: (now - chrono::Duration::minutes(1)).to_rfc3339(),
        expires_at: (now + chrono::Duration::days(30)).to_rfc3339(),
        identity_scheme: IDENTITY_SCHEME_SEMANTIC_V1.into(),
        permissions: PolicyPermissions::default(),
        scopes: vec![
            PolicyScope {
                id: "org-default".into(),
                kind: ScopeKind::Organization,
                selector: "acme".into(),
                priority: 0,
                enforcement: EnforcementMode::Block,
                rules: BTreeMap::new(),
                custom_rules: vec![],
                exceptions: vec![],
            },
            PolicyScope {
                id: "marts-select-star".into(),
                kind: ScopeKind::Path,
                selector: "models/marts/**".into(),
                priority: 10,
                enforcement: EnforcementMode::Block,
                rules: BTreeMap::from([(
                    "SQLCOST001".into(),
                    RulePolicy {
                        enabled: Some(false),
                        ..RulePolicy::default()
                    },
                )]),
                custom_rules: vec![],
                exceptions: vec![],
            },
        ],
    };
    let key = generate_key("root-2026", now).unwrap();
    let signed = sign_policy(&policy, &key).unwrap();
    let trust = TrustStoreV1 {
        version: 1,
        keys: vec![TrustedKeyV1 {
            key_id: key.key_id.clone(),
            algorithm: key.algorithm.clone(),
            public_key: key.public_key.clone(),
            valid_from: (now - chrono::Duration::days(1)).to_rfc3339(),
            valid_until: (now + chrono::Duration::days(365)).to_rfc3339(),
            revoked: false,
        }],
    };
    let bundle_path = temp.path().join("policy.json");
    let trust_store = temp.path().join("trust.json");
    fs::write(&bundle_path, serde_json::to_vec_pretty(&signed).unwrap()).unwrap();
    fs::write(&trust_store, serde_json::to_vec_pretty(&trust).unwrap()).unwrap();

    let config = ScanConfig {
        root: temp.path().to_path_buf(),
        paths: vec![temp.path().join("models")],
        fail_on: Some(Severity::Medium),
        signed_policy: SignedPolicyConfig {
            bundle_path: Some(bundle_path.clone()),
            trust_store_path: Some(trust_store),
            organization: Some("acme".into()),
            repository: Some("acme/warehouse".into()),
            required: true,
            ..SignedPolicyConfig::default()
        },
        ..ScanConfig::default()
    };
    let result = scan(&config).unwrap();

    let marts_select_star = result.diagnostics.iter().any(|diagnostic| {
        diagnostic.rule_id == "SQLCOST001"
            && diagnostic
                .path
                .to_string_lossy()
                .replace('\\', "/")
                .contains("models/marts/orders.sql")
    });
    let intermediate_select_star = result.diagnostics.iter().any(|diagnostic| {
        diagnostic.rule_id == "SQLCOST001"
            && diagnostic
                .path
                .to_string_lossy()
                .replace('\\', "/")
                .contains("models/intermediate/customers.sql")
    });
    assert!(
        !marts_select_star,
        "path-scoped disable should suppress SQLCOST001 in models/marts"
    );
    assert!(
        intermediate_select_star,
        "SQLCOST001 should still fire for models/intermediate when only marts path is disabled"
    );

    // ponytail: severity override uses the same per-file path; marts High, intermediate keeps default Medium.
    policy.scopes[1].rules = BTreeMap::from([(
        "SQLCOST001".into(),
        RulePolicy {
            severity: Some(Severity::High),
            ..RulePolicy::default()
        },
    )]);
    let signed = sign_policy(&policy, &key).unwrap();
    fs::write(&bundle_path, serde_json::to_vec_pretty(&signed).unwrap()).unwrap();
    let result = scan(&config).unwrap();
    let marts_severity = result
        .diagnostics
        .iter()
        .find(|diagnostic| {
            diagnostic.rule_id == "SQLCOST001"
                && diagnostic
                    .path
                    .to_string_lossy()
                    .replace('\\', "/")
                    .contains("models/marts/orders.sql")
        })
        .map(|diagnostic| diagnostic.severity);
    let intermediate_severity = result
        .diagnostics
        .iter()
        .find(|diagnostic| {
            diagnostic.rule_id == "SQLCOST001"
                && diagnostic
                    .path
                    .to_string_lossy()
                    .replace('\\', "/")
                    .contains("models/intermediate/customers.sql")
        })
        .map(|diagnostic| diagnostic.severity);
    assert_eq!(marts_severity, Some(Severity::High));
    assert_eq!(intermediate_severity, Some(Severity::Medium));
}
