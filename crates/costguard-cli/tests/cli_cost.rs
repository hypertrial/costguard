mod common;

use common::{costguard_command, fixture, git};
use std::fs;

#[test]
fn pr_cost_increase_gate_without_dollar_pricing_is_configuration_error() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let root = tempdir.path();
    fs::create_dir_all(root.join("models")).expect("create models");
    fs::write(root.join("models/a.sql"), "select 1 as id\n").expect("write model");
    git(root, &["init"]);
    git(root, &["config", "user.email", "costguard@example.com"]);
    git(root, &["config", "user.name", "Costguard Test"]);
    git(root, &["add", "."]);
    git(root, &["commit", "-m", "initial"]);
    fs::write(root.join("models/a.sql"), "select 2 as id\n").expect("modify model");

    let output = costguard_command()
        .args(["pr", "--base", "HEAD", "--fail-on-pr-cost-increase", "100"])
        .current_dir(root)
        .output()
        .expect("run PR cost gate");
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("[cost.pricing].model"), "{stderr}");
}

#[test]
fn scan_cost_estimate_json_includes_cost_fields() {
    let fixture = fixture("cost_estimate");
    let output = costguard_command()
        .current_dir(&fixture)
        .arg("scan")
        .arg(".")
        .arg("--manifest")
        .arg("target/manifest.json")
        .arg("--format")
        .arg("json")
        .output()
        .expect("run costguard");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("\"cost_estimate\""),
        "expected cost_estimate in output:\n{stdout}"
    );
    assert!(stdout.contains("\"relative_index\""), "{stdout}");
    assert!(stdout.contains("\"savings_p50_usd_per_month\""), "{stdout}");
}

#[test]
fn scan_cost_summary_json_includes_cost_block() {
    let fixture = fixture("cost_estimate");
    let output = costguard_command()
        .current_dir(&fixture)
        .arg("scan")
        .arg(".")
        .arg("--manifest")
        .arg("target/manifest.json")
        .arg("--format")
        .arg("json")
        .output()
        .expect("run costguard");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("\"cost\""),
        "expected cost block:\n{stdout}"
    );
    assert!(stdout.contains("\"project_p50_usd\""), "{stdout}");
    assert!(stdout.contains("\"model_id\""), "{stdout}");
}

#[test]
fn cost_command_renders_project_report() {
    let fixture = fixture("cost_estimate");
    let output = costguard_command()
        .current_dir(&fixture)
        .arg("cost")
        .arg("report")
        .arg(".")
        .arg("--manifest")
        .arg("target/manifest.json")
        .output()
        .expect("run costguard cost");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Cost summary") || stdout.contains("cost prioritization summary"),
        "expected cost report output:\n{stdout}"
    );
}

#[test]
fn scan_cost_delta_gate_fails_when_threshold_exceeded() {
    let fixture = fixture("cost_estimate");
    let output = costguard_command()
        .current_dir(&fixture)
        .arg("scan")
        .arg(".")
        .arg("--manifest")
        .arg("target/manifest.json")
        .arg("--fail-on")
        .arg("critical")
        .arg("--fail-on-cost-delta")
        .arg("1")
        .output()
        .expect("run costguard");
    assert_eq!(
        output.status.code(),
        Some(1),
        "expected cost delta gate failure:\n{}",
        String::from_utf8_lossy(&output.stdout)
    );
}

#[test]
fn pr_cost_delta_gate_passes_for_clean_model_addition() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let root = tempdir.path();
    fs::create_dir_all(root.join("models/marts")).expect("create models");
    fs::create_dir_all(root.join("target")).expect("target");
    fs::copy(
        fixture("cost_estimate").join("target/catalog.json"),
        root.join("target/catalog.json"),
    )
    .expect("copy catalog");
    fs::write(root.join("models/marts/a.sql"), "select 1 as id\n").expect("write a");
    fs::write(
        root.join("dbt_project.yml"),
        "name: test\nversion: 1.0.0\nconfig-version: 2\nmodel-paths: [\"models\"]\n",
    )
    .expect("write dbt project");
    fs::write(
        root.join("costguard.toml"),
        r#"warehouse = "bigquery"

[cost]
enabled = true
default_runs_per_month = 30

[cost.pricing]
model = "scan"
usd_per_tb = 6.25

[cost.inputs]
catalog = "target/catalog.json"

[cost.sources."raw.events"]
bytes = "100GB"
"#,
    )
    .expect("write cost config");
    fs::write(
        root.join("target/manifest.json"),
        r#"{
  "metadata": {
    "dbt_schema_version": "https://schemas.getdbt.com/dbt/manifest/v12.json",
    "project_name": "test"
  },
  "nodes": {
    "model.test.a": {
      "resource_type": "model",
      "name": "a",
      "package_name": "test",
      "original_file_path": "models/marts/a.sql",
      "path": "marts/a.sql",
      "config": { "materialized": "table" },
      "depends_on": { "nodes": ["source.test.raw.events"] }
    }
  },
  "sources": {
    "source.test.raw.events": {
      "resource_type": "source",
      "source_name": "raw",
      "name": "events",
      "identifier": "events"
    }
  }
}"#,
    )
    .expect("write manifest");

    git(root, &["init"]);
    git(root, &["config", "user.email", "costguard@example.com"]);
    git(root, &["config", "user.name", "Costguard Test"]);
    git(root, &["add", "."]);
    git(root, &["commit", "-m", "initial"]);

    fs::write(
        root.join("models/marts/b.sql"),
        "{{ config(materialized='table', partition_by='id') }}\nselect 1 as id\n",
    )
    .expect("write b");
    fs::write(
        root.join("target/manifest.json"),
        r#"{
  "metadata": {
    "dbt_schema_version": "https://schemas.getdbt.com/dbt/manifest/v12.json",
    "project_name": "test"
  },
  "nodes": {
    "model.test.a": {
      "resource_type": "model",
      "name": "a",
      "package_name": "test",
      "original_file_path": "models/marts/a.sql",
      "path": "marts/a.sql",
      "config": { "materialized": "table" },
      "depends_on": { "nodes": ["source.test.raw.events"] }
    },
    "model.test.b": {
      "resource_type": "model",
      "name": "b",
      "package_name": "test",
      "original_file_path": "models/marts/b.sql",
      "path": "marts/b.sql",
      "config": { "materialized": "table", "partition_by": "id" },
      "depends_on": { "nodes": [] }
    }
  },
  "sources": {
    "source.test.raw.events": {
      "resource_type": "source",
      "source_name": "raw",
      "name": "events",
      "identifier": "events"
    }
  }
}"#,
    )
    .expect("update manifest");

    let output = costguard_command()
        .arg("pr")
        .arg("--base")
        .arg("HEAD")
        .arg("--format")
        .arg("json")
        .arg("--fail-on")
        .arg("critical")
        .arg("--fail-on-cost-delta")
        .arg("1")
        .current_dir(root)
        .output()
        .expect("run costguard");
    assert_eq!(
        output.status.code(),
        Some(0),
        "clean model addition should not trip cost gate:\n{}",
        String::from_utf8_lossy(&output.stdout)
    );
}

#[test]
fn min_cost_coverage_fails_when_mapped_spend_is_below_threshold() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let root = tempdir.path();
    fs::create_dir_all(root.join("models/marts")).expect("models");
    fs::create_dir_all(root.join("target")).expect("target");
    fs::copy(
        fixture("cost_estimate").join("target/catalog.json"),
        root.join("target/catalog.json"),
    )
    .expect("catalog");
    fs::copy(
        fixture("cost_estimate").join("observations.json"),
        root.join("observations.json"),
    )
    .expect("observations");
    fs::write(root.join("models/marts/a.sql"), "select 1 as id\n").expect("a");
    fs::write(root.join("models/marts/b.sql"), "select 2 as id\n").expect("b");
    fs::write(
        root.join("dbt_project.yml"),
        "name: test\nversion: 1.0.0\nconfig-version: 2\nmodel-paths: [\"models\"]\n",
    )
    .expect("dbt project");
    fs::write(
        root.join("costguard.toml"),
        r#"warehouse = "bigquery"

[cost]
enabled = true

[cost.pricing]
model = "scan"
usd_per_tb = 6.25

[cost.inputs]
catalog = "target/catalog.json"
observations = "observations.json"

[cost.sources."raw.events"]
bytes = "100GB"
"#,
    )
    .expect("cost config");
    fs::write(
        root.join("target/manifest.json"),
        r#"{
  "metadata": {
    "dbt_schema_version": "https://schemas.getdbt.com/dbt/manifest/v12.json",
    "project_name": "test"
  },
  "nodes": {
    "model.test.a": {
      "resource_type": "model",
      "name": "a",
      "package_name": "test",
      "original_file_path": "models/marts/a.sql",
      "path": "marts/a.sql",
      "config": { "materialized": "table" },
      "depends_on": { "nodes": [] }
    },
    "model.test.b": {
      "resource_type": "model",
      "name": "b",
      "package_name": "test",
      "original_file_path": "models/marts/b.sql",
      "path": "marts/b.sql",
      "config": { "materialized": "table" },
      "depends_on": { "nodes": [] }
    }
  },
  "sources": {}
}"#,
    )
    .expect("manifest");

    let output = costguard_command()
        .current_dir(root)
        .args([
            "scan",
            ".",
            "--manifest",
            "target/manifest.json",
            "--min-cost-coverage",
            "0.8",
        ])
        .output()
        .expect("scan");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        output.status.code(),
        Some(1),
        "expected cost coverage gate failure:\n{stdout}"
    );
    assert!(
        stdout.contains("cost_coverage"),
        "expected cost_coverage violation:\n{stdout}"
    );
}

#[test]
fn min_cost_coverage_passes_when_observations_cover_models() {
    let fixture = fixture("cost_estimate");
    let output = costguard_command()
        .current_dir(&fixture)
        .args([
            "scan",
            ".",
            "--manifest",
            "target/manifest.json",
            "--fail-on",
            "critical",
            "--min-cost-coverage",
            "0.8",
        ])
        .output()
        .expect("scan");
    assert_eq!(
        output.status.code(),
        Some(0),
        "expected pass:\n{}",
        String::from_utf8_lossy(&output.stdout)
    );
}

#[test]
fn min_cost_coverage_is_noop_without_cost_or_threshold() {
    let fixture = fixture("corpus/jinja_heavy");
    let output = costguard_command()
        .current_dir(&fixture)
        .args(["scan", ".", "--format", "json"])
        .output()
        .expect("scan");
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.contains("cost_coverage"),
        "unexpected cost coverage violation:\n{stdout}"
    );
}
