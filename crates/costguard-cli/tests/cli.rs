use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_costguard"))
}

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
}

#[test]
fn scan_text_reports_mvp_diagnostics() {
    let output = Command::new(bin())
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
        "SQLCOST005",
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
    let output = Command::new(bin())
        .arg("scan")
        .arg(fixture("dbt_incremental"))
        .arg("--format")
        .arg("json")
        .output()
        .expect("run costguard");
    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("\"diagnostics\""));
    assert!(stdout.contains("\"rule_id\""));
}

#[test]
fn rules_command_lists_rules() {
    let output = Command::new(bin())
        .arg("rules")
        .output()
        .expect("run costguard");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("SQLCOST001"));
    assert!(stdout.contains("SQLCOST015"));
}

#[test]
fn invalid_config_flag_exits_with_configuration_code() {
    let output = Command::new(bin())
        .arg("scan")
        .arg("--fail-on")
        .arg("not-a-severity")
        .output()
        .expect("run costguard");
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("configuration error"));
}

#[test]
fn scan_uses_config_paths_when_no_paths_are_passed() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let root = tempdir.path();
    fs::write(
        root.join("costguard.toml"),
        format!(
            "[scan]\npaths = [\"{}\"]\n",
            fixture("dbt_incremental/models/marts/fct_sessions.sql").display()
        ),
    )
    .expect("write config");

    let output = Command::new(bin())
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
    let output = Command::new(bin())
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
    fs::write(
        root.join("costguard.toml"),
        format!(
            "[scan]\npaths = [\"{}\"]\nignore = [\"{}\"]\n",
            fixture("dbt_incremental").display(),
            fixture("dbt_incremental/models/staging").display()
        ),
    )
    .expect("write config");

    let output = Command::new(bin())
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
