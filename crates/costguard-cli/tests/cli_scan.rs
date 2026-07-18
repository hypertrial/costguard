mod common;

use common::{costguard_command, fixture, git};
use std::fs;

#[test]
fn scan_text_reports_mvp_diagnostics() {
    let output = costguard_command()
        .arg("scan")
        .arg(fixture("dbt_incremental"))
        .arg("--warehouse")
        .arg("snowflake")
        .output()
        .expect("run costguard");
    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    for rule in [
        "SQLCOST001",
        "SQLCOST002",
        "SQLCOST003",
        "SQLCOST004",
        "SQLCOST008",
        "SQLCOST010",
        "SQLCOST011",
        "SQLCOST012",
        "SQLCOST013",
    ] {
        assert!(stdout.contains(rule), "missing {rule} in:\n{stdout}");
    }
}

#[test]
fn scan_json_outputs_diagnostics_array() {
    let output = costguard_command()
        .arg("scan")
        .arg(fixture("dbt_incremental"))
        .arg("--format")
        .arg("json")
        .output()
        .expect("run costguard");
    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("\"diagnostics\""));
    assert!(stdout.contains("\"metrics\""));
    assert!(stdout.contains("\"rule_id\""));
}

#[test]
fn scan_github_outputs_annotations() {
    let output = costguard_command()
        .arg("scan")
        .arg(fixture("dbt_incremental"))
        .arg("--format")
        .arg("github")
        .output()
        .expect("run costguard");
    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("::error file="), "{stdout}");
    assert!(stdout.contains("title=SQLCOST"), "{stdout}");
}

#[test]
fn scan_markdown_outputs_pr_summary_shape() {
    let output = costguard_command()
        .arg("scan")
        .arg(fixture("corpus/incremental_missing"))
        .arg("--format")
        .arg("markdown")
        .output()
        .expect("run costguard");
    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("# Costguard failed this PR"), "{stdout}");
    assert!(stdout.contains("## Diagnostics"), "{stdout}");
    assert!(stdout.contains("SQLCOST005"), "{stdout}");
}

#[test]
fn sealed_rocky_dsl_runs_shared_sql_rules_without_fake_source_location() {
    let repo = tempfile::tempdir().unwrap();
    fs::create_dir_all(repo.path().join("models/marts")).unwrap();
    fs::create_dir_all(repo.path().join("target")).unwrap();
    fs::write(repo.path().join("rocky.toml"), "version = 1\n").unwrap();
    fs::write(
        repo.path().join("models/marts/orders.rocky"),
        "from raw_orders\nselect *\n",
    )
    .unwrap();
    git(repo.path(), &["init"]);
    git(repo.path(), &["config", "user.email", "test@example.com"]);
    git(repo.path(), &["config", "user.name", "Test"]);
    git(
        repo.path(),
        &["add", "rocky.toml", "models/marts/orders.rocky"],
    );
    git(repo.path(), &["commit", "-m", "fixture"]);
    fs::write(
        repo.path().join("target/rocky-compile.json"),
        serde_json::to_string(&serde_json::json!({
            "version": "0.9.0",
            "command": "compile",
            "has_errors": false,
            "models_detail": [{
                "name": "orders",
                "strategy": {"type": "view"},
                "target": {"catalog": "lake", "schema": "mart", "table": "orders"},
                "depends_on": [],
                "tags": {"owner": "@finance"}
            }],
            "expanded_sql": {"orders": "select * from raw_orders"}
        }))
        .unwrap(),
    )
    .unwrap();

    let capture = costguard_command()
        .current_dir(repo.path())
        .args(["rocky", "capture", "--compile", "target/rocky-compile.json"])
        .output()
        .unwrap();
    assert!(
        capture.status.success(),
        "{}",
        String::from_utf8_lossy(&capture.stderr)
    );

    let scan = costguard_command()
        .current_dir(repo.path())
        .args(["scan", "--format", "json", "--fail-on", "low"])
        .output()
        .unwrap();
    assert_eq!(
        scan.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&scan.stdout)
    );
    let payload: serde_json::Value = serde_json::from_slice(&scan.stdout).unwrap();
    assert_eq!(payload["metrics"]["counts"]["sql"], 1);
    let finding = payload["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|diagnostic| diagnostic["rule_id"] == "SQLCOST001")
        .unwrap();
    assert_eq!(finding["path"], "models/marts/orders.rocky");
    assert_eq!(finding["source_provenance"], "compiled_unmapped");
    assert_eq!(finding["line"], 1);
    assert!(finding["compiled_line"].as_u64().is_some());

    let github = costguard_command()
        .current_dir(repo.path())
        .args(["scan", "--format", "github", "--fail-on", "low"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&github.stdout);
    assert!(stdout.contains("compiled SQL location"), "{stdout}");
    assert!(stdout.contains("models/marts/orders.rocky"), "{stdout}");
    assert!(
        !stdout.contains("file=models/marts/orders.rocky"),
        "{stdout}"
    );
}

#[test]
fn rocky_capture_uses_configured_paths_by_default() {
    let repo = tempfile::tempdir().unwrap();
    fs::create_dir_all(repo.path().join("config")).unwrap();
    fs::create_dir_all(repo.path().join("rocky_models")).unwrap();
    fs::create_dir_all(repo.path().join("target")).unwrap();
    fs::write(
        repo.path().join("costguard.toml"),
        "[rocky]\nconfig_path = \"config/rocky.toml\"\nmodels_dir = \"rocky_models\"\nartifact_path = \"artifacts/rocky.json\"\n",
    )
    .unwrap();
    fs::write(repo.path().join("config/rocky.toml"), "version = 1\n").unwrap();
    fs::write(repo.path().join("rocky_models/orders.sql"), "select 1\n").unwrap();
    git(repo.path(), &["init"]);
    git(repo.path(), &["config", "user.email", "test@example.com"]);
    git(repo.path(), &["config", "user.name", "Test"]);
    git(
        repo.path(),
        &[
            "add",
            "costguard.toml",
            "config/rocky.toml",
            "rocky_models/orders.sql",
        ],
    );
    git(repo.path(), &["commit", "-m", "fixture"]);
    fs::write(
        repo.path().join("target/compile.json"),
        serde_json::to_vec(&serde_json::json!({
            "version": "0.9.0",
            "command": "compile",
            "has_errors": false,
            "models_detail": [{
                "name": "orders",
                "strategy": {"type": "view"},
                "target": {"catalog": "lake", "schema": "mart", "table": "orders"},
                "depends_on": [],
                "tags": {}
            }],
            "expanded_sql": {"orders": "select 1"}
        }))
        .unwrap(),
    )
    .unwrap();

    let output = costguard_command()
        .current_dir(repo.path())
        .args(["rocky", "capture", "--compile", "target/compile.json"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(repo.path().join("artifacts/rocky.json").is_file());
}

#[test]
fn rocky_sql_fallback_runs_shared_rules_but_skips_dbt_configuration_rules() {
    let repo = tempfile::tempdir().unwrap();
    fs::create_dir_all(repo.path().join("models/marts")).unwrap();
    fs::write(repo.path().join("rocky.toml"), "version = 1\n").unwrap();
    fs::write(
        repo.path().join("models/marts/orders.sql"),
        "{{ config(materialized='incremental') }}\nselect * from raw_orders\n",
    )
    .unwrap();

    let output = costguard_command()
        .current_dir(repo.path())
        .args(["scan", "--format", "json", "--fail-on", "critical"])
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let payload: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let rule_ids = payload["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|diagnostic| diagnostic["rule_id"].as_str())
        .collect::<Vec<_>>();
    assert!(rule_ids.contains(&"SQLCOST001"), "{rule_ids:?}");
    assert!(rule_ids.contains(&"SQLCOST047"), "{rule_ids:?}");
    assert!(!rule_ids.contains(&"SQLCOST004"), "{rule_ids:?}");
    assert!(!rule_ids.contains(&"SQLCOST005"), "{rule_ids:?}");

    git(repo.path(), &["init"]);
    git(repo.path(), &["config", "user.email", "test@example.com"]);
    git(repo.path(), &["config", "user.name", "Test"]);
    git(
        repo.path(),
        &["add", "rocky.toml", "models/marts/orders.sql"],
    );
    git(repo.path(), &["commit", "-m", "base"]);
    let base = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(repo.path())
        .output()
        .unwrap();
    let base = String::from_utf8(base.stdout).unwrap().trim().to_string();
    fs::write(
        repo.path().join("models/marts/orders.sql"),
        "{{ config(materialized='incremental') }}\nselect * from raw_orders\n-- changed\n",
    )
    .unwrap();
    git(repo.path(), &["add", "models/marts/orders.sql"]);
    git(repo.path(), &["commit", "-m", "head"]);

    let output = costguard_command()
        .current_dir(repo.path())
        .args([
            "pr",
            "--base",
            &base,
            "--block-only-new=true",
            "--format",
            "json",
            "--fail-on",
            "low",
        ])
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let payload: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        payload["pr_summary"]["rocky_artifact_integrity"]["unclassified_head_findings"],
        1
    );
    for status in ["introduced", "regressed", "resolved", "unchanged"] {
        assert_eq!(payload["pr_summary"]["finding_delta"][status], 0);
    }
}

#[test]
fn incomplete_rocky_comparison_does_not_report_removed_base_findings() {
    let repo = tempfile::tempdir().unwrap();
    fs::create_dir_all(repo.path().join("models")).unwrap();
    fs::write(repo.path().join("rocky.toml"), "version = 1\n").unwrap();
    fs::write(
        repo.path().join("models/orders.sql"),
        "select * from raw_orders\n",
    )
    .unwrap();
    git(repo.path(), &["init"]);
    git(repo.path(), &["config", "user.email", "test@example.com"]);
    git(repo.path(), &["config", "user.name", "Test"]);
    git(repo.path(), &["add", "rocky.toml", "models/orders.sql"]);
    git(repo.path(), &["commit", "-m", "base"]);
    let base = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(repo.path())
        .output()
        .unwrap();
    let base = String::from_utf8(base.stdout).unwrap().trim().to_string();
    fs::remove_file(repo.path().join("models/orders.sql")).unwrap();
    git(repo.path(), &["add", "models/orders.sql"]);
    git(repo.path(), &["commit", "-m", "remove rocky model"]);

    let output = costguard_command()
        .current_dir(repo.path())
        .args([
            "pr",
            "--base",
            &base,
            "--block-only-new=true",
            "--format",
            "json",
            "--fail-on",
            "low",
        ])
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let payload: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        payload["pr_summary"]["rocky_artifact_integrity"]["comparison_complete"],
        false
    );
    assert_eq!(payload["pr_summary"]["finding_delta"]["resolved"], 0);
}

#[test]
fn mixed_dbt_and_rocky_projects_allow_namespaces_but_reject_source_conflicts() {
    let repo = tempfile::tempdir().unwrap();
    fs::create_dir_all(repo.path().join("models/marts")).unwrap();
    fs::create_dir_all(repo.path().join("rocky_models/marts")).unwrap();
    fs::create_dir_all(repo.path().join("target")).unwrap();
    fs::write(
        repo.path().join("costguard.toml"),
        "[rocky]\nmodels_dir = \"rocky_models\"\n",
    )
    .unwrap();
    fs::write(repo.path().join("dbt_project.yml"), "name: mixed\n").unwrap();
    fs::write(repo.path().join("rocky.toml"), "version = 1\n").unwrap();
    fs::write(
        repo.path().join("models/marts/orders.sql"),
        "select * from raw.dbt_orders\n",
    )
    .unwrap();
    fs::write(
        repo.path().join("rocky_models/marts/orders.rocky"),
        "from raw_orders\nselect *\n",
    )
    .unwrap();
    git(repo.path(), &["init"]);
    git(repo.path(), &["config", "user.email", "test@example.com"]);
    git(repo.path(), &["config", "user.name", "Test"]);
    git(repo.path(), &["add", "."]);
    git(repo.path(), &["commit", "-m", "fixture"]);
    fs::write(
        repo.path().join("target/rocky-compile.json"),
        serde_json::to_vec(&serde_json::json!({
            "version": "0.9.0",
            "command": "compile",
            "has_errors": false,
            "models_detail": [{
                "name": "orders",
                "strategy": {"type": "view"},
                "target": {"catalog": "lake", "schema": "mart", "table": "orders"},
                "depends_on": [],
                "tags": {}
            }],
            "expanded_sql": {"orders": "select * from raw.rocky_orders"}
        }))
        .unwrap(),
    )
    .unwrap();
    let capture = costguard_command()
        .current_dir(repo.path())
        .args([
            "rocky",
            "capture",
            "--compile",
            "target/rocky-compile.json",
            "--models-dir",
            "rocky_models",
        ])
        .output()
        .unwrap();
    assert!(
        capture.status.success(),
        "{}",
        String::from_utf8_lossy(&capture.stderr)
    );

    let scan = costguard_command()
        .current_dir(repo.path())
        .args(["scan", "--format", "json", "--fail-on", "critical"])
        .output()
        .unwrap();
    assert_eq!(
        scan.status.code(),
        Some(0),
        "{}\n{}",
        String::from_utf8_lossy(&scan.stdout),
        String::from_utf8_lossy(&scan.stderr)
    );
    let payload: serde_json::Value = serde_json::from_slice(&scan.stdout).unwrap();
    let select_star = payload["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|diagnostic| diagnostic["rule_id"] == "SQLCOST001")
        .collect::<Vec<_>>();
    assert_eq!(select_star.len(), 2, "{select_star:?}");

    fs::write(
        repo.path().join("target/manifest.json"),
        serde_json::to_vec(&serde_json::json!({
            "nodes": {
                "model.pkg.orders": {
                    "resource_type": "model",
                    "name": "orders",
                    "original_file_path": "rocky_models/marts/orders.rocky",
                    "compiled_code": "select * from raw.rocky_orders"
                }
            }
        }))
        .unwrap(),
    )
    .unwrap();
    let conflict = costguard_command()
        .current_dir(repo.path())
        .args(["scan", "--format", "json", "--fail-on", "critical"])
        .output()
        .unwrap();
    assert_eq!(
        conflict.status.code(),
        Some(2),
        "{}\n{}",
        String::from_utf8_lossy(&conflict.stdout),
        String::from_utf8_lossy(&conflict.stderr)
    );
    assert!(
        String::from_utf8_lossy(&conflict.stderr)
            .contains("both claim rocky_models/marts/orders.rocky"),
        "{}",
        String::from_utf8_lossy(&conflict.stderr)
    );
}

#[test]
fn rocky_pr_compares_verified_base_and_reports_framework_details() {
    let repo = tempfile::tempdir().unwrap();
    fs::create_dir_all(repo.path().join("models/marts")).unwrap();
    fs::create_dir_all(repo.path().join("target")).unwrap();
    fs::write(repo.path().join("rocky.toml"), "version = 1\n").unwrap();
    fs::write(
        repo.path().join("models/marts/orders.sql"),
        "select id from raw_orders\n",
    )
    .unwrap();
    git(repo.path(), &["init"]);
    git(repo.path(), &["config", "user.email", "test@example.com"]);
    git(repo.path(), &["config", "user.name", "Test"]);
    git(
        repo.path(),
        &["add", "rocky.toml", "models/marts/orders.sql"],
    );
    git(repo.path(), &["commit", "-m", "base"]);
    let base = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(repo.path())
        .output()
        .unwrap();
    let base = String::from_utf8(base.stdout).unwrap().trim().to_string();
    write_rocky_compile(repo.path(), "select id from raw_orders");
    capture_rocky(repo.path());
    fs::copy(
        repo.path().join("target/costguard-rocky.json"),
        repo.path().join("base-rocky.json"),
    )
    .unwrap();

    fs::write(
        repo.path().join("models/marts/orders.sql"),
        "select * from raw_orders\n",
    )
    .unwrap();
    git(repo.path(), &["add", "models/marts/orders.sql"]);
    git(repo.path(), &["commit", "-m", "head"]);
    write_rocky_compile(repo.path(), "select * from raw_orders");
    capture_rocky(repo.path());

    let output = costguard_command()
        .current_dir(repo.path())
        .args([
            "pr",
            "--base",
            &base,
            "--base-rocky-artifact",
            "base-rocky.json",
            "--format",
            "json",
            "--fail-on",
            "low",
        ])
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(1),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let payload: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        payload["pr_summary"]["rocky_artifact_integrity"]["comparison_complete"],
        true
    );
    assert_eq!(
        payload["pr_summary"]["changed_model_details"][0]["framework"],
        "rocky"
    );
    assert_eq!(payload["pr_summary"]["finding_delta"]["introduced"], 1);
    assert_eq!(
        payload["pr_summary"]["recommended_commands"][0]["framework"],
        "rocky"
    );

    let incomplete = costguard_command()
        .current_dir(repo.path())
        .args([
            "pr",
            "--base",
            &base,
            "--block-only-new=true",
            "--format",
            "json",
            "--fail-on",
            "low",
        ])
        .output()
        .unwrap();
    assert_eq!(incomplete.status.code(), Some(0));
    let payload: serde_json::Value = serde_json::from_slice(&incomplete.stdout).unwrap();
    assert_eq!(
        payload["pr_summary"]["rocky_artifact_integrity"]["comparison_complete"],
        false
    );
    assert_eq!(
        payload["pr_summary"]["rocky_artifact_integrity"]["unclassified_head_findings"],
        1
    );
    assert_eq!(payload["pr_summary"]["finding_delta"]["introduced"], 0);
    assert_eq!(payload["pr_summary"]["finding_delta"]["regressed"], 0);

    let strict = costguard_command()
        .current_dir(repo.path())
        .args([
            "pr",
            "--base",
            &base,
            "--analysis-policy",
            "strict",
            "--format",
            "json",
        ])
        .output()
        .unwrap();
    assert_eq!(strict.status.code(), Some(1));
    let payload: serde_json::Value = serde_json::from_slice(&strict.stdout).unwrap();
    assert!(payload["analysis"]["violations"]
        .as_array()
        .unwrap()
        .iter()
        .any(|violation| violation["code"] == "rocky_artifact_integrity"));
}

fn write_rocky_compile(root: &std::path::Path, sql: &str) {
    fs::write(
        root.join("target/rocky-compile.json"),
        serde_json::to_string(&serde_json::json!({
            "version": "0.9.0",
            "command": "compile",
            "has_errors": false,
            "models_detail": [{
                "name": "orders",
                "strategy": {"type": "view"},
                "target": {"catalog": "lake", "schema": "mart", "table": "orders"},
                "depends_on": [],
                "tags": {"owner": "@finance"}
            }],
            "expanded_sql": {"orders": sql}
        }))
        .unwrap(),
    )
    .unwrap();
}

fn capture_rocky(root: &std::path::Path) {
    let output = costguard_command()
        .current_dir(root)
        .args(["rocky", "capture", "--compile", "target/rocky-compile.json"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn pr_mode_scans_changed_files_but_uses_transitive_context() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let root = tempdir.path();
    fs::create_dir_all(root.join("models/marts")).expect("create models");
    fs::write(root.join("models/marts/a.sql"), "select 1 as id\n").expect("write a");
    fs::write(
        root.join("models/marts/b.sql"),
        "select id from {{ ref('a') }}\n",
    )
    .expect("write b");
    fs::write(
        root.join("models/marts/c.sql"),
        "select id from {{ ref('b') }}\n",
    )
    .expect("write c");
    fs::write(
        root.join("models/marts/unchanged_risky.sql"),
        "select * from {{ ref('a') }}\n",
    )
    .expect("write risky");
    fs::write(
        root.join("schema.yml"),
        "version: 2\nexposures:\n  - name: dashboard\n    depends_on:\n      - ref('c')\n",
    )
    .expect("write schema");

    git(root, &["init"]);
    git(root, &["config", "user.email", "costguard@example.com"]);
    git(root, &["config", "user.name", "Costguard Test"]);
    git(root, &["add", "."]);
    git(root, &["commit", "-m", "initial"]);
    fs::write(root.join("models/marts/a.sql"), "select 2 as id\n").expect("modify a");

    let output = costguard_command()
        .arg("pr")
        .arg("--base")
        .arg("HEAD")
        .arg("--format")
        .arg("json")
        .arg("--fail-on")
        .arg("critical")
        .current_dir(root)
        .output()
        .expect("run costguard");
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("\"pr_summary\""), "{stdout}");
    assert!(
        stdout.contains("\"changed_models\": [\n      \"a\"\n    ]"),
        "{stdout}"
    );
    assert!(stdout.contains("\"b\""), "{stdout}");
    assert!(stdout.contains("\"c\""), "{stdout}");
    assert!(stdout.contains("\"dashboard\""), "{stdout}");
    assert!(
        !stdout.contains("SQLCOST001"),
        "unchanged files should not emit diagnostics:\n{stdout}"
    );
}

#[test]
fn pr_block_only_new_supports_bare_true_and_false_overrides() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let root = tempdir.path();
    fs::create_dir_all(root.join("models")).expect("create models");
    let model = "{{ config(materialized='incremental', unique_key='id') }}\n\nselect id from raw_orders\n{% if is_incremental() %}\nwhere id > 1\n{% endif %}\n";
    fs::write(root.join("models/a.sql"), model).expect("write model");
    git(root, &["init"]);
    git(root, &["config", "user.email", "costguard@example.com"]);
    git(root, &["config", "user.name", "Costguard Test"]);
    git(root, &["add", "."]);
    git(root, &["commit", "-m", "initial"]);
    fs::write(
        root.join("models/a.sql"),
        format!("{model}-- unrelated edit\n"),
    )
    .expect("modify model");

    for flag in ["--block-only-new", "--block-only-new=true"] {
        let output = costguard_command()
            .args(["pr", "--base", "HEAD", "--fail-on", "high", flag])
            .current_dir(root)
            .output()
            .expect("run regression-only PR scan");
        assert_eq!(
            output.status.code(),
            Some(0),
            "{flag} should ignore unchanged high findings:\nstdout={}\nstderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let output = costguard_command()
        .args([
            "pr",
            "--base",
            "HEAD",
            "--fail-on",
            "high",
            "--block-only-new=false",
        ])
        .current_dir(root)
        .output()
        .expect("run all-findings PR scan");
    assert_eq!(
        output.status.code(),
        Some(1),
        "false override should retain all-findings enforcement:\n{}",
        String::from_utf8_lossy(&output.stdout)
    );
}

#[test]
fn pr_receipt_routes_owners_applies_waiver_and_reports_trend() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let root = tempdir.path();
    fs::create_dir_all(root.join("models/finance")).unwrap();
    fs::write(
        root.join("models/finance/orders.sql"),
        "select id from raw_orders\n",
    )
    .unwrap();
    git(root, &["init"]);
    git(root, &["config", "user.email", "costguard@example.com"]);
    git(root, &["config", "user.name", "Costguard Test"]);
    git(root, &["add", "."]);
    git(root, &["commit", "-m", "initial"]);
    fs::write(
        root.join("models/finance/orders.sql"),
        "select * from raw_orders cross join raw_customers\n",
    )
    .unwrap();
    fs::write(
        root.join("costguard.toml"),
        r#"[owners.paths]
"models/finance/**" = "@finance"

[gate]
fail_on = "high"
require_owner = true

[[waivers]]
id = "CG-1"
rule_id = "SQLCOST012"
path = "models/finance/**"
owner = "@finance"
reason = "approved migration"
ticket_url = "https://example.com/CG-1"
approver = "@lead"
created_at = "2026-01-01T00:00:00Z"
expires_at = "2099-01-01T00:00:00Z"
"#,
    )
    .unwrap();
    let receipt = root.join("receipt.json");
    let summary = root.join("summary.md");
    let output = costguard_command()
        .args(["pr", "--base", "HEAD", "--format", "json", "--cost"])
        .arg("--warehouse")
        .arg("bigquery")
        .arg("--summary-file")
        .arg(&summary)
        .arg("--receipt-file")
        .arg(&receipt)
        .current_dir(root)
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let payload: serde_json::Value = serde_json::from_slice(&fs::read(&receipt).unwrap()).unwrap();
    assert_eq!(payload["pr_summary"]["receipt_version"], 2);
    assert_eq!(
        payload["pr_summary"]["changed_model_details"][0]["owners"][0],
        "@finance"
    );
    assert_eq!(payload["pr_summary"]["gate_results"][0]["status"], "pass");
    let diagnostic = payload["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|diagnostic| diagnostic["rule_id"] == "SQLCOST012")
        .unwrap();
    assert_eq!(diagnostic["enforcement"], "excepted");
    assert_eq!(
        diagnostic["cost_estimate"]["prior_basis"],
        "warehouse-prior:bigquery"
    );
    assert!(fs::read_to_string(&summary).unwrap().contains("## Gates"));

    let next = root.join("next.json");
    let compared = costguard_command()
        .args(["pr", "--base", "HEAD", "--format", "json"])
        .arg("--compare-receipt")
        .arg(&receipt)
        .arg("--receipt-file")
        .arg(&next)
        .current_dir(root)
        .output()
        .unwrap();
    assert_eq!(compared.status.code(), Some(0));
    let compared: serde_json::Value = serde_json::from_slice(&fs::read(&next).unwrap()).unwrap();
    assert_eq!(compared["pr_summary"]["trend"]["diagnostics_delta"], 0);
    assert_eq!(compared["pr_summary"]["trend"]["high_findings_delta"], 0);

    let config = fs::read_to_string(root.join("costguard.toml"))
        .unwrap()
        .replace("2099-01-01T00:00:00Z", "2026-02-01T00:00:00Z");
    fs::write(root.join("costguard.toml"), config).unwrap();
    let expired = costguard_command()
        .args(["pr", "--base", "HEAD", "--format", "json"])
        .current_dir(root)
        .output()
        .unwrap();
    assert_eq!(expired.status.code(), Some(1));
    let expired: serde_json::Value = serde_json::from_slice(&expired.stdout).unwrap();
    assert!(expired["analysis"]["violations"]
        .as_array()
        .unwrap()
        .iter()
        .any(|violation| violation["code"] == "expired_waiver"));
}

#[test]
fn pr_mode_scans_newline_filename_without_bypass() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let root = tempdir.path();
    fs::create_dir_all(root.join("queries")).expect("queries");
    let path = root.join("queries/risky\nmodel.sql");
    fs::write(&path, "select 1 as id\n").expect("write initial");
    git(root, &["init"]);
    git(root, &["checkout", "-b", "main"]);
    git(root, &["config", "user.email", "costguard@example.com"]);
    git(root, &["config", "user.name", "Costguard Test"]);
    git(root, &["add", "."]);
    git(root, &["commit", "-m", "initial"]);
    git(root, &["checkout", "-b", "feature"]);
    fs::write(&path, "select * from a cross join b\n").expect("write risky");

    let output = costguard_command()
        .arg("pr")
        .arg("--base")
        .arg("main")
        .arg("--format")
        .arg("json")
        .arg("--analysis-policy")
        .arg("strict")
        .current_dir(root)
        .output()
        .expect("run costguard");
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(json["metrics"]["counts"]["sql"], 1);
    assert!(json["pr_summary"]["changed_files"]
        .as_array()
        .expect("changed files")
        .iter()
        .any(|changed| changed.as_str() == Some("queries/risky\nmodel.sql")));
}

#[test]
fn strict_analysis_requires_manifest_for_dbt_projects() {
    let output = costguard_command()
        .arg("scan")
        .arg(fixture("dbt_incremental"))
        .arg("--format")
        .arg("json")
        .arg("--analysis-policy")
        .arg("strict")
        .arg("--fail-on")
        .arg("critical")
        .output()
        .expect("run costguard");
    assert_eq!(output.status.code(), Some(1));
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(json["analysis"]["passed"], false);
    assert!(json["analysis"]["violations"]
        .as_array()
        .expect("violations")
        .iter()
        .any(|violation| violation["code"] == "manifest_required"));
}

#[test]
fn zero_runs_cost_config_is_configuration_error_not_panic() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    fs::create_dir_all(tempdir.path().join("models")).expect("models");
    fs::write(tempdir.path().join("models/model.sql"), "select 1\n").expect("model");
    fs::write(
        tempdir.path().join("costguard.toml"),
        "[cost]\nenabled = true\ndefault_runs_per_month = 0\n",
    )
    .expect("config");
    let output = costguard_command()
        .arg("scan")
        .arg("--cost")
        .current_dir(tempdir.path())
        .output()
        .expect("run costguard");
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("greater than zero"), "{stderr}");
    assert!(!stderr.contains("panicked"), "{stderr}");
}

#[test]
fn baseline_warehouse_mismatch_is_configuration_error() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let baseline = tempdir.path().join("baseline.json");
    fs::write(
        &baseline,
        r#"{"version":3,"identity_scheme":"semantic-v1","platform":"snowflake","generated_at":"2026-01-01T00:00:00Z","findings":[]}"#,
    )
    .expect("baseline");
    let output = costguard_command()
        .arg("scan")
        .arg(fixture("dbt_incremental"))
        .arg("--warehouse")
        .arg("bigquery")
        .arg("--baseline")
        .arg(&baseline)
        .output()
        .expect("run costguard");
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("does not match"));
}

#[test]
fn scan_uses_config_paths_when_no_paths_are_passed() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let root = tempdir.path();
    fs::create_dir_all(root.join("models/marts")).expect("models dir");
    fs::write(
        root.join("models/marts/fct_sessions.sql"),
        "select * from source_table\n",
    )
    .expect("write model");
    fs::write(
        root.join("costguard.toml"),
        "[scan]\npaths = [\"models/marts/fct_sessions.sql\"]\n",
    )
    .expect("write config");

    let output = costguard_command()
        .arg("scan")
        .arg("--fail-on")
        .arg("critical")
        .current_dir(root)
        .output()
        .expect("run costguard");
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("SQLCOST001"), "{stdout}");
    assert!(!stdout.contains("SQLCOST002"), "{stdout}");
}

#[test]
fn missing_manifest_flag_fails_with_configuration_code() {
    let output = costguard_command()
        .arg("scan")
        .arg(fixture("dbt_incremental"))
        .arg("--manifest")
        .arg("does-not-exist/manifest.json")
        .output()
        .expect("run costguard");
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("manifest path does not exist"));
}

#[test]
fn scan_config_ignore_excludes_paths() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let root = tempdir.path();
    fs::create_dir_all(root.join("models/marts")).expect("marts dir");
    fs::create_dir_all(root.join("models/staging")).expect("staging dir");
    fs::write(
        root.join("models/marts/fct_sessions.sql"),
        "select * from source_table\n",
    )
    .expect("write mart");
    fs::write(
        root.join("models/staging/stg_events.sql"),
        "select json_extract(payload, '$.a'), json_extract(payload, '$.b') from events\n",
    )
    .expect("write staging");
    fs::write(
        root.join("costguard.toml"),
        "[scan]\npaths = [\"models\"]\nignore = [\"models/staging\"]\n",
    )
    .expect("write config");

    let output = costguard_command()
        .arg("scan")
        .arg("--fail-on")
        .arg("critical")
        .current_dir(root)
        .output()
        .expect("run costguard");
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.contains("SQLCOST002"), "{stdout}");
    assert!(stdout.contains("SQLCOST001"), "{stdout}");
}

#[test]
fn scan_config_max_file_bytes_skips_large_files() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let root = tempdir.path();
    fs::create_dir_all(root.join("models")).expect("models dir");
    fs::write(root.join("models/large.sql"), "select 1\nselect 2\n").expect("write sql");
    fs::write(root.join("costguard.toml"), "[scan]\nmax_file_bytes = 4\n").expect("write config");

    let output = costguard_command()
        .arg("scan")
        .arg("--fail-on")
        .arg("critical")
        .current_dir(root)
        .output()
        .expect("run costguard");
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("SQLCOST026"), "{stdout}");
    assert!(!stdout.contains("SQLCOST001"), "{stdout}");
}

#[test]
fn pr_mode_fails_in_non_git_directory() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let root = tempdir.path();
    fs::write(root.join("model.sql"), "select 1\n").expect("write model");

    let output = costguard_command()
        .arg("pr")
        .current_dir(root)
        .output()
        .expect("run costguard");
    assert_eq!(output.status.code(), Some(3));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("not a git repository"), "{stderr}");
}

#[test]
fn pr_mode_fails_for_invalid_base_ref() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let root = tempdir.path();
    fs::create_dir_all(root.join("models")).expect("create models");
    fs::write(root.join("models/a.sql"), "select 1\n").expect("write model");
    git(root, &["init"]);
    git(root, &["config", "user.email", "costguard@example.com"]);
    git(root, &["config", "user.name", "Costguard Test"]);
    git(root, &["add", "."]);
    git(root, &["commit", "-m", "initial"]);

    let output = costguard_command()
        .arg("pr")
        .arg("--base")
        .arg("does-not-exist")
        .current_dir(root)
        .output()
        .expect("run costguard");
    assert_eq!(output.status.code(), Some(3));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("does-not-exist"), "{stderr}");
}

#[test]
fn scan_min_confidence_suppresses_low_confidence_high_severity() {
    let path = fixture("min_confidence_low_comma.sql");
    let without_floor = costguard_command()
        .arg("scan")
        .arg(&path)
        .arg("--warehouse")
        .arg("generic")
        .arg("--fail-on")
        .arg("med")
        .output()
        .expect("run costguard");
    assert_eq!(
        without_floor.status.code(),
        Some(1),
        "expected fail without min-confidence:\n{}",
        String::from_utf8_lossy(&without_floor.stdout)
    );

    let with_floor = costguard_command()
        .arg("scan")
        .arg(&path)
        .arg("--warehouse")
        .arg("generic")
        .arg("--fail-on")
        .arg("med")
        .arg("--min-confidence")
        .arg("high")
        .output()
        .expect("run costguard");
    assert_eq!(
        with_floor.status.code(),
        Some(0),
        "expected pass with min-confidence high:\n{}",
        String::from_utf8_lossy(&with_floor.stdout)
    );
}

#[test]
fn scan_min_confidence_filter_omits_low_confidence_from_json_output() {
    let path = fixture("min_confidence_low_comma.sql");
    let default_run = costguard_command()
        .arg("scan")
        .arg(&path)
        .arg("--warehouse")
        .arg("generic")
        .arg("--format")
        .arg("json")
        .output()
        .expect("run costguard");
    let default_json: serde_json::Value =
        serde_json::from_slice(&default_run.stdout).expect("json");
    let default_count = default_json["diagnostics"]
        .as_array()
        .map(|items| items.len())
        .unwrap_or(0);
    assert!(
        default_count > 0,
        "expected default scan to emit diagnostics: {default_json}"
    );

    let filtered_run = costguard_command()
        .arg("scan")
        .arg(&path)
        .arg("--warehouse")
        .arg("generic")
        .arg("--format")
        .arg("json")
        .arg("--min-confidence")
        .arg("high")
        .arg("--min-confidence-filter")
        .output()
        .expect("run costguard");
    let filtered_json: serde_json::Value =
        serde_json::from_slice(&filtered_run.stdout).expect("json");
    let filtered_count = filtered_json["diagnostics"]
        .as_array()
        .map(|items| items.len())
        .unwrap_or(0);
    assert_eq!(
        filtered_count, 0,
        "expected filtered scan to omit low-confidence diagnostics: {filtered_json}"
    );
}

#[test]
fn scan_sarif_outputs_valid_schema_fields() {
    let output = costguard_command()
        .arg("scan")
        .arg(fixture("dbt_incremental"))
        .arg("--format")
        .arg("sarif")
        .output()
        .expect("run costguard");
    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("\"version\": \"2.1.0\""));
    assert!(stdout.contains("\"name\": \"costguard\""));
    assert!(stdout.contains("\"ruleId\""));
}

#[test]
fn baseline_grandfathers_known_findings() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let baseline_path = tempdir.path().join("baseline.json");
    let fixture = fixture("dbt_incremental");

    let write = costguard_command()
        .arg("scan")
        .arg(&fixture)
        .arg("--warehouse")
        .arg("snowflake")
        .arg("--write-baseline")
        .arg(&baseline_path)
        .output()
        .expect("write baseline");
    assert_eq!(write.status.code(), Some(1));
    assert!(baseline_path.exists());

    let with_baseline = costguard_command()
        .arg("scan")
        .arg(&fixture)
        .arg("--warehouse")
        .arg("snowflake")
        .arg("--baseline")
        .arg(&baseline_path)
        .arg("--fail-on")
        .arg("high")
        .output()
        .expect("scan with baseline");
    assert_eq!(
        with_baseline.status.code(),
        Some(0),
        "expected pass when all findings are baselined:\n{}",
        String::from_utf8_lossy(&with_baseline.stdout)
    );
    let stdout = String::from_utf8_lossy(&with_baseline.stdout);
    assert!(stdout.contains("No diagnostics"), "{stdout}");
}
