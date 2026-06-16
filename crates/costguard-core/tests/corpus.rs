use costguard_core::{scan, ScanConfig};
use costguard_diagnostics::Severity;
use costguard_sql::Platform;
use serde::Deserialize;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize)]
struct CorpusManifest {
    case: Vec<CorpusCase>,
}

#[derive(Debug, Deserialize)]
struct CorpusCase {
    name: String,
    path: String,
    expect_rules: Vec<String>,
    forbid_rules: Vec<String>,
    platform: Option<String>,
    manifest_path: Option<String>,
}

fn corpus_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/corpus")
}

fn load_cases() -> Vec<CorpusCase> {
    let manifest_path = corpus_root().join("manifest.toml");
    let text = std::fs::read_to_string(&manifest_path).expect("read corpus manifest");
    toml::from_str::<CorpusManifest>(&text)
        .expect("parse corpus manifest")
        .case
}

fn parse_platform(value: Option<&str>) -> Platform {
    match value.unwrap_or("generic").to_ascii_lowercase().as_str() {
        "bigquery" | "bq" => Platform::BigQuery,
        "snowflake" => Platform::Snowflake,
        "trino" => Platform::Trino,
        "databricks" => Platform::Databricks,
        "redshift" => Platform::Redshift,
        "postgres" | "postgresql" => Platform::Postgres,
        "duckdb" => Platform::DuckDB,
        _ => Platform::Generic,
    }
}

fn rule_ids_for_case(case_path: &Path, case: &CorpusCase) -> HashSet<String> {
    let manifest_path = case
        .manifest_path
        .as_ref()
        .map(|relative| case_path.join(relative));
    let config = ScanConfig {
        root: case_path.to_path_buf(),
        paths: vec![PathBuf::from("models")],
        platform: parse_platform(case.platform.as_deref()),
        manifest_path,
        fail_on: Some(Severity::High),
        ..ScanConfig::default()
    };
    let result = scan(&config).expect("scan corpus case");
    result
        .diagnostics
        .into_iter()
        .map(|diagnostic| diagnostic.rule_id)
        .collect()
}

#[test]
fn corpus_cases_match_expected_rules() {
    for case in load_cases() {
        let case_path = corpus_root().join(&case.path);
        let rule_ids = rule_ids_for_case(&case_path, &case);
        for rule in &case.expect_rules {
            assert!(
                rule_ids.contains(rule),
                "case {} missing expected rule {rule}; got {:?}",
                case.name,
                rule_ids
            );
        }
        for rule in &case.forbid_rules {
            assert!(
                !rule_ids.contains(rule),
                "case {} unexpectedly emitted forbidden rule {rule}; got {:?}",
                case.name,
                rule_ids
            );
        }
    }
}

#[test]
fn compiled_manifest_parse_uses_compiled_code_for_metrics() {
    use costguard_sql::Platform;

    let case_path = corpus_root().join("compiled_manifest_parse");
    let config = ScanConfig {
        root: case_path.clone(),
        paths: vec![PathBuf::from("models")],
        platform: Platform::Trino,
        manifest_path: Some(case_path.join("target/manifest.json")),
        fail_on: Some(Severity::High),
        ..ScanConfig::default()
    };
    let result = scan(&config).expect("scan compiled manifest case");
    assert_eq!(result.metrics.sql_parse_total, 1);
    assert_eq!(result.metrics.sql_parse_failures, 0);
    assert_eq!(result.metrics.sql_parse_compiled_total, 1);
    assert_eq!(result.metrics.sql_parse_compiled_failures, 0);
}

#[test]
fn trino_parse_fixture_uses_compiled_normalization() {
    use costguard_sql::Platform;

    let case_path = corpus_root().join("trino_parse");
    let config = ScanConfig {
        root: case_path.clone(),
        paths: vec![PathBuf::from("models")],
        platform: Platform::Trino,
        manifest_path: Some(case_path.join("target/manifest.json")),
        fail_on: Some(Severity::High),
        ..ScanConfig::default()
    };
    let result = scan(&config).expect("scan trino parse case");
    assert_eq!(result.metrics.sql_parse_total, 1);
    assert_eq!(result.metrics.sql_parse_failures, 0);
    assert_eq!(result.metrics.sql_parse_compiled_total, 1);
    assert_eq!(result.metrics.sql_parse_compiled_failures, 0);
}

#[test]
fn spellbook_compiled_parse_fixture_has_zero_compiled_failures() {
    use costguard_sql::Platform;

    let case_path = corpus_root().join("spellbook_compiled_parse");
    let config = ScanConfig {
        root: case_path.clone(),
        paths: vec![PathBuf::from("models")],
        platform: Platform::Trino,
        manifest_path: Some(case_path.join("target/manifest.json")),
        fail_on: Some(Severity::High),
        ..ScanConfig::default()
    };
    let result = scan(&config).expect("scan spellbook compiled parse case");
    assert_eq!(result.metrics.sql_parse_total, 3);
    assert_eq!(result.metrics.sql_parse_compiled_total, 3);
    assert_eq!(result.metrics.sql_parse_compiled_failures, 0);
}

#[test]
fn skipped_file_emits_sqlcost026() {
    let case_path = corpus_root().join("file_skipped");
    let config = ScanConfig {
        root: case_path,
        paths: vec![PathBuf::from("models")],
        max_file_bytes: Some(4),
        fail_on: Some(Severity::High),
        ..ScanConfig::default()
    };
    let result = scan(&config).expect("scan file_skipped case");
    let rule_ids: std::collections::HashSet<_> = result
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.rule_id.as_str())
        .collect();
    assert!(
        rule_ids.contains("SQLCOST026"),
        "expected SQLCOST026; got {rule_ids:?}"
    );
    assert!(
        !rule_ids.contains("SQLCOST025"),
        "SQLCOST025 must not be used for skipped files; got {rule_ids:?}"
    );
}

#[test]
fn synthetic_project_scan_smoke() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let output = std::process::Command::new("python3")
        .arg(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../scripts/generate_synthetic_dbt.py"),
        )
        .arg(tempdir.path())
        .arg("--models")
        .arg("100")
        .output()
        .expect("run synthetic generator");
    assert!(
        output.status.success(),
        "generator failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let config = ScanConfig {
        root: tempdir.path().to_path_buf(),
        fail_on: Some(Severity::Critical),
        ..ScanConfig::default()
    };
    scan(&config).expect("scan synthetic project");
}
