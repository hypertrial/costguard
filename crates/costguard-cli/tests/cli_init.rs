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
    assert!(workflow.contains("fail-on-pr-cost-increase"));
    let config = fs::read_to_string(root.join("costguard.toml")).unwrap();
    assert!(config.contains("block_only_new = true"));
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
