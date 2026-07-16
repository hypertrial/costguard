mod common;

use common::costguard_command;
use std::fs;

#[test]
fn init_writes_workflow_and_config() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    fs::write(root.join("dbt_project.yml"), "name: demo\nprofile: dev\n").unwrap();
    fs::write(
        root.join("profiles.yml"),
        r#"
dev:
  target: prod
  outputs:
    prod:
      type: snowflake
"#,
    )
    .unwrap();

    let output = costguard_command()
        .args(["init"])
        .current_dir(root)
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(root.join(".github/workflows/costguard.yml").is_file());
    assert!(root.join("costguard.toml").is_file());
    let workflow = fs::read_to_string(root.join(".github/workflows/costguard.yml")).unwrap();
    assert!(workflow.contains("warehouse: snowflake"));
    assert!(workflow.contains("block-only-new: true"));
    assert!(workflow.contains("pull-requests: write"));
    assert!(workflow.contains("pr-comment: true"));
    assert!(workflow.contains("github-token: ${{ github.token }}"));
    assert!(workflow.contains("fail-on-pr-cost-increase"));
    let config = fs::read_to_string(root.join("costguard.toml")).unwrap();
    assert!(config.contains("block_only_new = true"));
    assert!(String::from_utf8_lossy(&output.stdout).contains("Costguard doctor"));
}

#[test]
fn init_skips_existing_without_force() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    fs::write(root.join("costguard.toml"), "warehouse = \"generic\"\n").unwrap();

    let first = costguard_command()
        .args(["init", "--no-workflow"])
        .current_dir(root)
        .output()
        .unwrap();
    assert_eq!(first.status.code(), Some(0));

    let second = costguard_command()
        .args(["init", "--no-workflow"])
        .current_dir(root)
        .output()
        .unwrap();
    assert_eq!(second.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&second.stdout);
    assert!(stdout.contains("skipped"));
}

#[test]
fn init_force_overwrites_and_warehouse_override() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    fs::write(root.join("costguard.toml"), "warehouse = \"generic\"\n").unwrap();

    let output = costguard_command()
        .args(["init", "--no-workflow", "--force", "--warehouse", "trino"])
        .current_dir(root)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    let config = fs::read_to_string(root.join("costguard.toml")).unwrap();
    assert!(config.contains("warehouse = \"trino\""));
}

#[test]
fn init_local_duckdb_profile_supports_dbt_dir() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();

    let output = costguard_command()
        .args(["init", "--profile", "local-duckdb", "--dbt-dir", "dbt"])
        .current_dir(root)
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let workflow = fs::read_to_string(root.join(".github/workflows/costguard.yml")).unwrap();
    assert!(workflow.contains("working-directory: dbt"));
    assert!(workflow.contains("warehouse: duckdb"));
    assert!(!workflow.contains("@vv"));

    let config = fs::read_to_string(root.join("dbt/costguard.toml")).unwrap();
    assert!(config.contains("warehouse = \"duckdb\""));
    assert!(config.contains("[scan]\npaths = [\"models\"]"));
    assert!(config.contains("# policy = \"strict\""));
}

#[test]
fn init_local_duckdb_profile_rejects_other_warehouse() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();

    let output = costguard_command()
        .args(["init", "--profile", "local-duckdb", "--warehouse", "trino"])
        .current_dir(root)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(3));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("requires --warehouse duckdb"), "{stderr}");
}
