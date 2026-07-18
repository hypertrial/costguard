mod common;

use common::costguard_command;
use std::fs;

#[test]
fn rules_command_lists_rules() {
    let output = costguard_command()
        .arg("rules")
        .output()
        .expect("run costguard");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("SQLCOST001"));
    assert!(stdout.contains("SQLCOST015"));
    assert!(stdout.contains("SQLCOST044"));
}

#[test]
fn version_reports_workspace_version() {
    let output = costguard_command()
        .arg("--version")
        .output()
        .expect("run costguard");
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "costguard 2.7.0"
    );
}

#[test]
fn version_propagates_to_every_subcommand() {
    for subcommand in ["scan", "pr", "explain", "rules"] {
        let output = costguard_command()
            .args([subcommand, "--version"])
            .output()
            .expect("run costguard subcommand version");
        assert!(output.status.success(), "{subcommand}");
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            format!("costguard-{subcommand} 2.7.0"),
            "{subcommand}"
        );
    }
}

#[test]
fn invalid_config_flag_exits_with_configuration_code() {
    let output = costguard_command()
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
fn malformed_config_is_classified_consistently_across_commands() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    fs::write(tempdir.path().join("costguard.toml"), "[scan\n").expect("write malformed config");
    let commands = [
        vec!["scan"],
        vec!["explain", "model.sql"],
        vec!["pr", "--base", "HEAD"],
        vec!["cost", "report"],
        vec!["doctor"],
        vec!["rocky", "capture", "--compile", "missing.json"],
    ];

    for args in commands {
        let output = costguard_command()
            .args(&args)
            .current_dir(tempdir.path())
            .output()
            .expect("run costguard");
        assert_eq!(
            output.status.code(),
            Some(2),
            "{}\nstdout:\n{}\nstderr:\n{}",
            args.join(" "),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.starts_with("error: configuration error: "),
            "{}: {stderr}",
            args.join(" ")
        );
        assert_eq!(
            stderr.matches("configuration error").count(),
            1,
            "{}: {stderr}",
            args.join(" ")
        );
    }
}

#[test]
fn unknown_config_field_exits_with_configuration_code() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    fs::write(
        tempdir.path().join("costguard.toml"),
        "[scan]\nunknown = true\n",
    )
    .expect("write config");
    let output = costguard_command()
        .arg("scan")
        .current_dir(tempdir.path())
        .output()
        .expect("run costguard");
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("unknown field"));
}

#[test]
fn unknown_config_section_exits_with_configuration_code() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    fs::write(
        tempdir.path().join("costguard.toml"),
        "[unknown]\nvalue = true\n",
    )
    .expect("write config");
    let output = costguard_command()
        .arg("scan")
        .current_dir(tempdir.path())
        .output()
        .expect("run costguard");
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("unknown field"));
}

#[test]
fn unknown_rule_id_exits_with_configuration_code() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    fs::write(
        tempdir.path().join("costguard.toml"),
        "[rules.SQLCOST999]\nenabled = false\n",
    )
    .expect("write config");
    let output = costguard_command()
        .arg("scan")
        .current_dir(tempdir.path())
        .output()
        .expect("run costguard");
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("unknown rule id"));
}

#[test]
fn unknown_rule_setting_exits_with_configuration_code() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    fs::write(
        tempdir.path().join("costguard.toml"),
        "[rules.SQLCOST001]\nunknown = true\n",
    )
    .expect("write config");
    let output = costguard_command()
        .arg("scan")
        .current_dir(tempdir.path())
        .output()
        .expect("run costguard");
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("unknown field"));
}
