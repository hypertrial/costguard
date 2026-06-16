use costguard_core::{scan, AnalysisConfig, AnalysisPolicy, Platform, ScanConfig};
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

fn write_stale_fixture(root: &std::path::Path) {
    std::fs::create_dir_all(root.join("models/marts")).expect("create models");
    std::fs::create_dir_all(root.join("target")).expect("create target");
    std::fs::write(root.join("models/marts/orders.sql"), "select 1\n").expect("write model");
    std::fs::write(
        root.join("target/manifest.json"),
        r#"{"nodes":{"model.pkg.orders":{"resource_type":"model","name":"orders","original_file_path":"models/marts/orders.sql","compiled_code":"select 1"}}}"#,
    )
    .expect("write manifest");

    let past = SystemTime::now() - Duration::from_secs(3600);
    let manifest_file = std::fs::OpenOptions::new()
        .write(true)
        .open(root.join("target/manifest.json"))
        .expect("open manifest");
    manifest_file
        .set_modified(past)
        .expect("set manifest mtime");
    std::fs::write(root.join("models/marts/orders.sql"), "select 2\n").expect("rewrite model");
}

#[test]
fn stale_manifest_emits_sqlcost045() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_stale_fixture(temp.path());

    let config = ScanConfig {
        root: temp.path().to_path_buf(),
        paths: vec![PathBuf::from("models")],
        platform: Platform::Generic,
        manifest_path: Some(temp.path().join("target/manifest.json")),
        fail_on: None,
        ..ScanConfig::default()
    };
    let result = scan(&config).expect("scan");
    assert!(
        result
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.rule_id == "SQLCOST045"),
        "expected SQLCOST045; got {:?}",
        result
            .diagnostics
            .iter()
            .map(|diagnostic| &diagnostic.rule_id)
            .collect::<Vec<_>>()
    );
}

#[test]
fn strict_fails_on_stale_manifest() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_stale_fixture(temp.path());

    let config = ScanConfig {
        root: temp.path().to_path_buf(),
        paths: vec![PathBuf::from("models")],
        platform: Platform::Generic,
        manifest_path: Some(temp.path().join("target/manifest.json")),
        fail_on: None,
        analysis: AnalysisConfig {
            policy: AnalysisPolicy::Strict,
            ..AnalysisConfig::default()
        },
        ..ScanConfig::default()
    };
    let result = scan(&config).expect("scan");
    assert!(
        !result.analysis.passed,
        "strict scan should fail on stale manifest"
    );
    assert!(
        result
            .analysis
            .violations
            .iter()
            .any(|violation| violation.code == "manifest_stale"),
        "expected manifest_stale violation; got {:?}",
        result.analysis.violations
    );
}

#[test]
fn fresh_manifest_emits_no_stale_warning() {
    let temp = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(temp.path().join("models/marts")).expect("create models");
    std::fs::create_dir_all(temp.path().join("target")).expect("create target");
    std::fs::write(temp.path().join("models/marts/orders.sql"), "select 1\n").expect("write model");
    std::fs::write(
        temp.path().join("target/manifest.json"),
        r#"{"nodes":{"model.pkg.orders":{"resource_type":"model","name":"orders","original_file_path":"models/marts/orders.sql","compiled_code":"select 1"}}}"#,
    )
    .expect("write manifest");

    let future = SystemTime::now() + Duration::from_secs(3600);
    let manifest_file = std::fs::OpenOptions::new()
        .write(true)
        .open(temp.path().join("target/manifest.json"))
        .expect("open manifest");
    manifest_file
        .set_modified(future)
        .expect("set manifest mtime");

    let config = ScanConfig {
        root: temp.path().to_path_buf(),
        paths: vec![PathBuf::from("models")],
        platform: Platform::Generic,
        manifest_path: Some(temp.path().join("target/manifest.json")),
        fail_on: None,
        ..ScanConfig::default()
    };
    let result = scan(&config).expect("scan");
    assert!(
        !result
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.rule_id == "SQLCOST045"),
        "fresh manifest should not emit SQLCOST045"
    );
}
