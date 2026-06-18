#[path = "common/mod.rs"]
mod common;

use common::{repo_root, run_git};
use costguard_core::{apply_file_config, load_config, scan, ScanConfig};
use costguard_sql::Platform;
use std::fs;
use std::path::{Path, PathBuf};

fn write_cost_repo(root: &Path, manifest: &str) {
    fs::create_dir_all(root.join("models/marts")).expect("models");
    fs::create_dir_all(root.join("target")).expect("target");
    fs::copy(
        repo_root().join("tests/fixtures/cost_estimate/target/catalog.json"),
        root.join("target/catalog.json"),
    )
    .expect("catalog");
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
    .expect("cost config");
    fs::write(root.join("target/manifest.json"), manifest).expect("manifest");
}

fn scan_pr(root: &Path, base: &str) -> costguard_core::ScanResult {
    let file_config = load_config(root).expect("load config");
    let base_config = ScanConfig {
        root: root.to_path_buf(),
        changed_only: true,
        base_branch: Some(base.into()),
        paths: vec![root.to_path_buf()],
        platform: Platform::BigQuery,
        manifest_path: Some(PathBuf::from("target/manifest.json")),
        ..ScanConfig::default()
    };
    let config = apply_file_config(base_config, file_config).expect("apply config");
    scan(&config).expect("scan")
}

fn manifest_one_model() -> &'static str {
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
      "config": { "materialized": "table", "partition_by": "id" },
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
}"#
}

fn manifest_two_models() -> &'static str {
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
      "config": { "materialized": "table", "partition_by": "id" },
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
}"#
}

#[test]
fn pr_impact_add_reports_single_model_introduced() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let root = tempdir.path();
    write_cost_repo(root, manifest_one_model());
    fs::write(
        root.join("models/marts/a.sql"),
        "{{ config(materialized='table', partition_by='id') }}\nselect 1 as id\n",
    )
    .expect("write a");

    run_git(root, &["init"]);
    run_git(root, &["config", "user.email", "costguard@example.com"]);
    run_git(root, &["config", "user.name", "Costguard Test"]);
    run_git(root, &["add", "."]);
    run_git(root, &["commit", "-m", "initial"]);

    fs::write(
        root.join("models/marts/b.sql"),
        "{{ config(materialized='table', partition_by='id') }}\nselect 1 as id\n",
    )
    .expect("write b");
    fs::write(root.join("target/manifest.json"), manifest_two_models()).expect("manifest");

    let result = scan_pr(root, "HEAD");
    let impact = result
        .cost_summary
        .as_ref()
        .and_then(|summary| summary.pr_impact.as_ref())
        .expect("pr impact");
    let head_total = result
        .cost_summary
        .as_ref()
        .and_then(|summary| summary.current_cost.monthly_p50)
        .expect("head total");
    let introduced = impact.introduced.monthly_p50.expect("introduced");
    let net = impact.net.monthly_p50.expect("net");
    assert!(introduced > 0.0, "added model should introduce cost");
    assert!(net > 0.0, "net impact should reflect addition");
    assert!(
        net < head_total,
        "net {net} should be less than full head total {head_total}"
    );
}

#[test]
fn pr_impact_delete_reports_avoided_cost() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let root = tempdir.path();
    write_cost_repo(root, manifest_two_models());
    fs::write(
        root.join("models/marts/a.sql"),
        "{{ config(materialized='table', partition_by='id') }}\nselect 1 as id\n",
    )
    .expect("write a");
    fs::write(
        root.join("models/marts/b.sql"),
        "{{ config(materialized='table', partition_by='id') }}\nselect 1 as id\n",
    )
    .expect("write b");

    run_git(root, &["init"]);
    run_git(root, &["config", "user.email", "costguard@example.com"]);
    run_git(root, &["config", "user.name", "Costguard Test"]);
    run_git(root, &["add", "."]);
    run_git(root, &["commit", "-m", "initial"]);

    fs::remove_file(root.join("models/marts/b.sql")).expect("delete b");
    fs::write(root.join("target/manifest.json"), manifest_one_model()).expect("manifest");

    let result = scan_pr(root, "HEAD");
    let impact = result
        .cost_summary
        .as_ref()
        .and_then(|summary| summary.pr_impact.as_ref())
        .expect("pr impact");
    let avoided = impact.avoided.monthly_p50.expect("avoided");
    assert!(avoided > 0.0, "deleted model should avoid cost");
}

#[test]
fn pr_impact_rename_does_not_double_count_models() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let root = tempdir.path();
    write_cost_repo(root, manifest_one_model());
    fs::write(
        root.join("models/marts/a.sql"),
        "{{ config(materialized='table', partition_by='id') }}\nselect 1 as id\n",
    )
    .expect("write a");

    run_git(root, &["init"]);
    run_git(root, &["config", "user.email", "costguard@example.com"]);
    run_git(root, &["config", "user.name", "Costguard Test"]);
    run_git(root, &["add", "."]);
    run_git(root, &["commit", "-m", "initial"]);

    run_git(
        root,
        &["mv", "models/marts/a.sql", "models/marts/renamed.sql"],
    );
    let renamed_manifest = r#"{
  "metadata": {
    "dbt_schema_version": "https://schemas.getdbt.com/dbt/manifest/v12.json",
    "project_name": "test"
  },
  "nodes": {
    "model.test.renamed": {
      "resource_type": "model",
      "name": "renamed",
      "package_name": "test",
      "original_file_path": "models/marts/renamed.sql",
      "path": "marts/renamed.sql",
      "config": { "materialized": "table", "partition_by": "id" },
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
}"#;
    fs::write(root.join("target/manifest.json"), renamed_manifest).expect("manifest");

    let result = scan_pr(root, "HEAD");
    let impact = result
        .cost_summary
        .as_ref()
        .and_then(|summary| summary.pr_impact.as_ref())
        .expect("pr impact");
    let head_total = result
        .cost_summary
        .as_ref()
        .and_then(|summary| summary.current_cost.monthly_p50)
        .expect("head total");
    let net = impact.net.monthly_p50.expect("net");
    assert!(
        net.abs() < head_total,
        "rename should not report full-project reintroduction; net={net} head_total={head_total}"
    );
}

#[test]
fn pr_impact_sql_edit_changes_efficiency_not_full_reintroduction() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let root = tempdir.path();
    write_cost_repo(root, manifest_one_model());
    fs::write(
        root.join("models/marts/a.sql"),
        "{{ config(materialized='table', partition_by='id') }}\nselect 1 as id\n",
    )
    .expect("write a");

    run_git(root, &["init"]);
    run_git(root, &["config", "user.email", "costguard@example.com"]);
    run_git(root, &["config", "user.name", "Costguard Test"]);
    run_git(root, &["add", "."]);
    run_git(root, &["commit", "-m", "initial"]);

    fs::write(
        root.join("models/marts/a.sql"),
        "{{ config(materialized='table', partition_by='id') }}\nselect * from {{ source('raw', 'events') }}\n",
    )
    .expect("write risky a");

    let result = scan_pr(root, "HEAD");
    let impact = result
        .cost_summary
        .as_ref()
        .and_then(|summary| summary.pr_impact.as_ref())
        .expect("pr impact");
    let introduced = impact.introduced.monthly_p50.unwrap_or(0.0);
    let efficiency = impact.efficiency.monthly_p50.expect("efficiency");
    assert!(
        result
            .diagnostics
            .iter()
            .any(|diag| diag.rule_id == "SQLCOST001"),
        "expected select-star finding on edited model"
    );
    assert!(
        introduced < impact.net.monthly_p50.unwrap_or(0.0).max(efficiency),
        "sql edit should shift efficiency, not only reintroduce volume"
    );
}

#[test]
fn pr_impact_reports_blast_radius_for_downstream_chain() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let root = tempdir.path();
    let chain_manifest = r#"{
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
      "config": { "materialized": "table", "partition_by": "id" },
      "depends_on": { "nodes": ["source.test.raw.events"] }
    },
    "model.test.b": {
      "resource_type": "model",
      "name": "b",
      "package_name": "test",
      "original_file_path": "models/marts/b.sql",
      "path": "marts/b.sql",
      "config": { "materialized": "table", "partition_by": "id" },
      "depends_on": { "nodes": ["model.test.a"] }
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
}"#;
    write_cost_repo(root, chain_manifest);
    fs::write(
        root.join("models/marts/a.sql"),
        "{{ config(materialized='table', partition_by='id') }}\nselect 1 as id\n",
    )
    .expect("write a");
    fs::write(
        root.join("models/marts/b.sql"),
        "{{ config(materialized='table', partition_by='id') }}\nselect * from {{ ref('a') }}\n",
    )
    .expect("write b");

    run_git(root, &["init"]);
    run_git(root, &["config", "user.email", "costguard@example.com"]);
    run_git(root, &["config", "user.name", "Costguard Test"]);
    run_git(root, &["add", "."]);
    run_git(root, &["commit", "-m", "initial"]);

    fs::write(
        root.join("models/marts/a.sql"),
        "{{ config(materialized='table', partition_by='id') }}\nselect * from {{ source('raw', 'events') }}\n",
    )
    .expect("write risky a");

    let result = scan_pr(root, "HEAD");
    let impact = result
        .cost_summary
        .as_ref()
        .and_then(|summary| summary.pr_impact.as_ref())
        .expect("pr impact");
    let blast = impact
        .blast_radius
        .monthly_p50
        .expect("blast radius should have downstream cost");
    assert!(
        blast > 0.0,
        "changed upstream should expose downstream spend"
    );
}

#[test]
fn pr_impact_manifest_config_change_shifts_base_cost() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let root = tempdir.path();
    write_cost_repo(root, manifest_one_model());
    fs::write(
        root.join("models/marts/a.sql"),
        "{{ config(materialized='table', partition_by='id') }}\nselect 1 as id\n",
    )
    .expect("write a");

    run_git(root, &["init"]);
    run_git(root, &["config", "user.email", "costguard@example.com"]);
    run_git(root, &["config", "user.name", "Costguard Test"]);
    run_git(root, &["add", "."]);
    run_git(root, &["commit", "-m", "initial"]);

    let unpartitioned_manifest = r#"{
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
}"#;
    fs::write(root.join("target/manifest.json"), unpartitioned_manifest).expect("manifest");

    let result = scan_pr(root, "HEAD");
    let impact = result
        .cost_summary
        .as_ref()
        .and_then(|summary| summary.pr_impact.as_ref())
        .expect("pr impact");
    assert!(
        impact.efficiency.monthly_p50.unwrap_or(0.0).abs() > 0.0
            || impact.volume.monthly_p50.unwrap_or(0.0).abs() > 0.0,
        "manifest config change should shift base-vs-head cost decomposition"
    );
}
