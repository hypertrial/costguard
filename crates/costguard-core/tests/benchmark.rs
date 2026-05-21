use costguard_core::{scan, ScanConfig};
use costguard_diagnostics::Severity;
use costguard_platform::Platform;
use serde::Deserialize;
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize)]
struct BaselineFile {
    target: String,
    warehouse: String,
    metrics: BaselineMetrics,
    #[serde(default)]
    expect_rules: Vec<String>,
    #[serde(default)]
    forbid_rules: Vec<String>,
    thresholds: BaselineThresholds,
}

#[derive(Debug, Deserialize)]
struct BaselineMetrics {
    #[allow(dead_code)]
    counts: BaselineCounts,
    #[allow(dead_code)]
    sql_parse_total: usize,
    sql_parse_failures: usize,
    #[allow(dead_code)]
    diagnostics_by_rule: BTreeMap<String, usize>,
    #[allow(dead_code)]
    diagnostics_by_severity: BTreeMap<String, usize>,
}

#[derive(Debug, Deserialize)]
struct BaselineCounts {
    #[allow(dead_code)]
    sql: usize,
    #[allow(dead_code)]
    yaml: usize,
    #[allow(dead_code)]
    python: usize,
    #[allow(dead_code)]
    manifest: usize,
    #[allow(dead_code)]
    other: usize,
}

#[derive(Debug, Deserialize, Default)]
struct BaselineThresholds {
    max_sql_parse_failures: Option<usize>,
    #[allow(dead_code)]
    max_parse_failure_rate: Option<f64>,
    #[allow(dead_code)]
    max_parse_failure_delta: Option<usize>,
    exact_diagnostics_by_rule: Option<BTreeMap<String, usize>>,
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn baselines_dir() -> PathBuf {
    repo_root().join("tests/benchmarks/baselines")
}

fn load_vendored_baselines() -> Vec<BaselineFile> {
    baselines_dir()
        .read_dir()
        .expect("read baselines dir")
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension().is_some_and(|ext| ext == "json")
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("real_world__"))
        })
        .map(|path| {
            let text = std::fs::read_to_string(&path).expect("read baseline");
            serde_json::from_str(&text).unwrap_or_else(|err| {
                panic!("parse baseline {}: {err}", path.display());
            })
        })
        .collect()
}

fn scan_fixture(fixture: &Path, warehouse: Platform) -> costguard_core::ScanResult {
    let config = ScanConfig {
        root: repo_root(),
        paths: vec![fixture.to_path_buf()],
        platform: warehouse,
        manifest_path: fixture
            .join("target/manifest.json")
            .exists()
            .then_some(fixture.join("target/manifest.json")),
        fail_on: Some(Severity::Critical),
        ..ScanConfig::default()
    };
    scan(&config).expect("scan fixture")
}

fn compare_baseline(baseline: &BaselineFile, result: &costguard_core::ScanResult) {
    let actual = &result.metrics;
    let max_parse_failures = baseline
        .thresholds
        .max_sql_parse_failures
        .unwrap_or(baseline.metrics.sql_parse_failures);
    assert!(
        actual.sql_parse_failures <= max_parse_failures,
        "target {} parse failures {} > allowed {}",
        baseline.target,
        actual.sql_parse_failures,
        max_parse_failures
    );

    if let Some(exact) = &baseline.thresholds.exact_diagnostics_by_rule {
        for (rule, expected_count) in exact {
            let actual_count = actual.diagnostics_by_rule.get(rule).copied().unwrap_or(0);
            assert_eq!(
                actual_count, *expected_count,
                "target {} rule {rule} count mismatch",
                baseline.target
            );
        }
    }

    let actual_rules: HashSet<&str> = actual
        .diagnostics_by_rule
        .keys()
        .map(String::as_str)
        .collect();
    for rule in &baseline.expect_rules {
        assert!(
            actual_rules.contains(rule.as_str()),
            "target {} missing expected rule {rule}; got {:?}",
            baseline.target,
            actual_rules
        );
    }
    for rule in &baseline.forbid_rules {
        assert!(
            !actual_rules.contains(rule.as_str()),
            "target {} unexpectedly emitted forbidden rule {rule}; got {:?}",
            baseline.target,
            actual_rules
        );
    }
}

#[test]
fn vendored_baselines_match() {
    for baseline in load_vendored_baselines() {
        let fixture = repo_root()
            .join("tests/fixtures")
            .join(baseline.target.replace('/', std::path::MAIN_SEPARATOR_STR));
        let warehouse = baseline
            .warehouse
            .parse::<Platform>()
            .expect("baseline warehouse");
        let result = scan_fixture(&fixture, warehouse);
        compare_baseline(&baseline, &result);
    }
}

#[test]
#[ignore = "requires network clone; run with COSTGUARD_BENCHMARK=1"]
fn external_repo_benchmark() {
    if std::env::var("COSTGUARD_BENCHMARK").ok().as_deref() != Some("1") {
        eprintln!("skipping external_repo_benchmark; set COSTGUARD_BENCHMARK=1 to enable");
        return;
    }

    let script = repo_root().join("scripts/benchmark_external_repo.py");
    for repo in ["jaffle-shop", "spellbook"] {
        let output = std::process::Command::new("python3")
            .arg(&script)
            .arg("--repo")
            .arg(repo)
            .output()
            .unwrap_or_else(|err| panic!("run benchmark script for {repo}: {err}"));
        assert!(
            output.status.success(),
            "benchmark for {repo} failed:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
