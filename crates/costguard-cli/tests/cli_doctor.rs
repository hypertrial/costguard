mod common;

use common::{costguard_command, git};
use std::fs;

fn initialize_repository(root: &std::path::Path) {
    git(root, &["init"]);
    git(root, &["config", "user.email", "costguard@example.com"]);
    git(root, &["config", "user.name", "Costguard Test"]);
}

fn write_workflow(root: &std::path::Path) {
    let directory = root.join(".github/workflows");
    fs::create_dir_all(&directory).unwrap();
    fs::write(
        directory.join("costguard.yml"),
        r#"on: pull_request
permissions:
  contents: read
  pull-requests: write
jobs:
  costguard:
    steps:
      - uses: actions/checkout@0000000000000000000000000000000000000000
        with:
          fetch-depth: 0
      - uses: hypertrial/costguard/.github/actions/costguard@v2
        with:
          min-confidence: high
          block-only-new: true
          pr-comment: true
          github-token: ${{ github.token }}
"#,
    )
    .unwrap();
}

#[test]
fn doctor_reports_ready_project() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    initialize_repository(root);
    fs::create_dir(root.join("models")).unwrap();
    fs::write(root.join("models/model.sql"), "select 1 as id\n").unwrap();
    fs::write(root.join("costguard.toml"), "warehouse = \"duckdb\"\n").unwrap();
    write_workflow(root);
    git(root, &["add", "."]);
    git(root, &["commit", "-m", "initial"]);

    let output = costguard_command()
        .arg("doctor")
        .current_dir(root)
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("[pass] committed HEAD"), "{stdout}");
    assert!(stdout.contains("[warn] cost coverage"), "{stdout}");
    assert!(stdout.contains("Ready:"), "{stdout}");
}

#[test]
fn doctor_uses_dbt_dir_but_repository_workflow() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    initialize_repository(root);
    fs::create_dir_all(root.join("analytics/models")).unwrap();
    fs::write(root.join("analytics/models/model.sql"), "select 1 as id\n").unwrap();
    fs::write(
        root.join("analytics/costguard.toml"),
        "warehouse = \"duckdb\"\n",
    )
    .unwrap();
    write_workflow(root);
    git(root, &["add", "."]);
    git(root, &["commit", "-m", "initial"]);

    let output = costguard_command()
        .args(["doctor", "--dbt-dir", "analytics"])
        .current_dir(root)
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("recommended PR contract"));
}

#[test]
fn doctor_exit_codes_distinguish_blockers_and_configuration() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("model.sql"), "select 1\n").unwrap();
    let blocked = costguard_command()
        .arg("doctor")
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert_eq!(blocked.status.code(), Some(1));

    fs::write(
        temp.path().join("costguard.toml"),
        "warehouse = \"invalid\"\n",
    )
    .unwrap();
    let invalid = costguard_command()
        .arg("doctor")
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert_eq!(invalid.status.code(), Some(2));
}

#[test]
fn doctor_strict_policy_blocks_missing_manifest() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    initialize_repository(root);
    fs::create_dir_all(root.join("models")).unwrap();
    fs::write(root.join("models/model.sql"), "select 1 as id\n").unwrap();
    fs::write(root.join("dbt_project.yml"), "name: demo\n").unwrap();
    fs::write(
        root.join("costguard.toml"),
        "warehouse = \"duckdb\"\n[analysis]\npolicy = \"strict\"\n",
    )
    .unwrap();
    write_workflow(root);
    git(root, &["add", "."]);
    git(root, &["commit", "-m", "initial"]);

    let output = costguard_command()
        .arg("doctor")
        .current_dir(root)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("[fail] analysis policy"), "{stdout}");
    assert!(stdout.contains("run 'dbt compile'"), "{stdout}");
}

#[test]
fn doctor_warns_for_missing_manifest_in_standard_mode() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    initialize_repository(root);
    fs::create_dir_all(root.join("models")).unwrap();
    fs::write(root.join("models/model.sql"), "select 1 as id\n").unwrap();
    fs::write(root.join("dbt_project.yml"), "name: demo\n").unwrap();
    fs::write(root.join("costguard.toml"), "warehouse = \"duckdb\"\n").unwrap();
    write_workflow(root);
    git(root, &["add", "."]);
    git(root, &["commit", "-m", "initial"]);

    let output = costguard_command()
        .arg("doctor")
        .current_dir(root)
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("[warn] dbt manifest"), "{stdout}");
    assert!(stdout.contains("[warn] cost coverage"), "{stdout}");
}

#[test]
fn doctor_runtime_scan_failure_exits_three() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    initialize_repository(root);
    fs::write(root.join("model.sql"), "select 1 as id\n").unwrap();
    fs::write(
        root.join("costguard.toml"),
        "warehouse = \"duckdb\"\n[cost.inputs]\nobservations = \"missing.json\"\n",
    )
    .unwrap();
    git(root, &["add", "."]);
    git(root, &["commit", "-m", "initial"]);

    let output = costguard_command()
        .arg("doctor")
        .current_dir(root)
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&output.stderr).contains("missing.json"));
}

#[test]
fn doctor_blocks_when_the_configured_finding_gate_fails() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    initialize_repository(root);
    fs::create_dir(root.join("models")).unwrap();
    fs::write(
        root.join("models/model.sql"),
        "select * from source where id not in (select id from excluded)\n",
    )
    .unwrap();
    fs::write(root.join("costguard.toml"), "warehouse = \"duckdb\"\n").unwrap();
    write_workflow(root);
    git(root, &["add", "."]);
    git(root, &["commit", "-m", "initial"]);

    let output = costguard_command()
        .arg("doctor")
        .current_dir(root)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("[fail] configured policy"), "{stdout}");
}

#[test]
fn help_describes_commands_and_examples() {
    let root = tempfile::tempdir().unwrap();
    let top = costguard_command()
        .arg("--help")
        .current_dir(root.path())
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&top.stdout);
    assert!(text.contains("doctor"), "{text}");
    assert!(text.contains("Check whether a project is ready"), "{text}");

    let scan = costguard_command()
        .args(["scan", "--help"])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&scan.stdout);
    assert!(text.contains("costguard scan models --cost"), "{text}");
    assert!(text.contains("Minimum severity"), "{text}");
}
