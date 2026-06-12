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
    assert!(stdout.contains("\"metrics\""));
    assert!(stdout.contains("\"rule_id\""));
}

#[test]
fn scan_github_outputs_annotations() {
    let output = Command::new(bin())
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
    let output = Command::new(bin())
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

    let output = Command::new(bin())
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
fn version_reports_workspace_version() {
    let output = Command::new(bin())
        .arg("--version")
        .output()
        .expect("run costguard");
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "costguard 1.0.0"
    );
}

#[test]
fn version_propagates_to_every_subcommand() {
    for subcommand in ["scan", "pr", "explain", "rules"] {
        let output = Command::new(bin())
            .args([subcommand, "--version"])
            .output()
            .expect("run costguard subcommand version");
        assert!(output.status.success(), "{subcommand}");
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            format!("costguard-{subcommand} 1.0.0"),
            "{subcommand}"
        );
    }
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
fn unknown_config_field_exits_with_configuration_code() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    fs::write(
        tempdir.path().join("costguard.toml"),
        "[scan]\nunknown = true\n",
    )
    .expect("write config");
    let output = Command::new(bin())
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
    let output = Command::new(bin())
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
    let output = Command::new(bin())
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
    let output = Command::new(bin())
        .arg("scan")
        .current_dir(tempdir.path())
        .output()
        .expect("run costguard");
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("unknown field"));
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

#[test]
fn scan_config_max_file_bytes_skips_large_files() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let root = tempdir.path();
    fs::create_dir_all(root.join("models")).expect("models dir");
    fs::write(root.join("models/large.sql"), "select 1\nselect 2\n").expect("write sql");
    fs::write(root.join("costguard.toml"), "[scan]\nmax_file_bytes = 4\n").expect("write config");

    let output = Command::new(bin())
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

fn git(root: &std::path::Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .expect("run git");
    assert!(
        output.status.success(),
        "git {} failed\nstdout:\n{}\nstderr:\n{}",
        args.join(" "),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn pr_mode_fails_in_non_git_directory() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let root = tempdir.path();
    fs::write(root.join("model.sql"), "select 1\n").expect("write model");

    let output = Command::new(bin())
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

    let output = Command::new(bin())
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
    let without_floor = Command::new(bin())
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

    let with_floor = Command::new(bin())
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
fn scan_sarif_outputs_valid_schema_fields() {
    let output = Command::new(bin())
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

    let write = Command::new(bin())
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

    let with_baseline = Command::new(bin())
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
