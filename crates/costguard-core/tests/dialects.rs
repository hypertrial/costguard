use costguard_core::{scan, Platform, ScanConfig};
use std::path::PathBuf;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn production_dialects_have_zero_parse_failures() {
    for platform in [
        Platform::Generic,
        Platform::Snowflake,
        Platform::BigQuery,
        Platform::Trino,
    ] {
        let result = scan_fixture(platform);
        assert_eq!(parse_total(&result), 1, "{platform}");
        assert_eq!(parse_failures(&result), 0, "{platform}");
        assert!(result.diagnostics.is_empty(), "{platform}");
    }
}

#[test]
fn preview_dialects_have_deterministic_smoke_coverage() {
    for platform in [
        Platform::Databricks,
        Platform::Redshift,
        Platform::Postgres,
        Platform::DuckDB,
    ] {
        let result = scan_fixture(platform);
        assert_eq!(parse_total(&result), 1, "{platform}");
        assert!(parse_failures(&result) <= 1, "{platform}");
    }
}

fn parse_total(result: &costguard_core::ScanResult) -> usize {
    result.metrics.sql_parse_total + result.metrics.sql_parse_other_total
}

fn parse_failures(result: &costguard_core::ScanResult) -> usize {
    result.metrics.sql_parse_failures + result.metrics.sql_parse_other_failures
}

fn scan_fixture(platform: Platform) -> costguard_core::ScanResult {
    let root = repo_root();
    scan(&ScanConfig {
        root: root.clone(),
        paths: vec![root
            .join("tests/fixtures/dialects")
            .join(format!("{}.sql", platform.label()))],
        platform,
        ..ScanConfig::default()
    })
    .expect("scan dialect fixture")
}
