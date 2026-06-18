use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_costguard"))
}

fn costguard_command() -> Command {
    let mut command = Command::new(bin());
    command.current_dir(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."));
    command
}

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
}

#[test]
fn policy_cli_compiles_signs_and_verifies() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("policy.toml");
    let compiled = temp.path().join("policy.json");
    let private_key = temp.path().join("private.json");
    let trust = temp.path().join("trust.json");
    let bundle = temp.path().join("bundle.json");
    let now = chrono::Utc::now();
    fs::write(
        &source,
        format!(
            r#"schema_version = 2
id = "org-default"
version = "2026.06"
organization = "acme"
issued_at = "{}"
expires_at = "{}"
identity_scheme = "semantic-v1"

[[scopes]]
id = "org"
kind = "organization"
selector = "acme"
priority = 0
enforcement = "block"
"#,
            (now - chrono::Duration::minutes(1)).to_rfc3339(),
            (now + chrono::Duration::days(30)).to_rfc3339()
        ),
    )
    .unwrap();

    let keygen = costguard_command()
        .args(["policy", "keygen", "root-2026", "--private-key"])
        .arg(&private_key)
        .arg("--trust-store")
        .arg(&trust)
        .output()
        .unwrap();
    assert!(
        keygen.status.success(),
        "{}",
        String::from_utf8_lossy(&keygen.stderr)
    );

    let commands = [
        vec![source.clone(), compiled.clone()],
        vec![compiled.clone(), private_key.clone(), bundle.clone()],
        vec![bundle.clone(), trust.clone()],
    ];
    for (subcommand, paths) in ["compile", "sign", "verify"].into_iter().zip(commands) {
        let output = costguard_command()
            .args(["policy", subcommand])
            .args(paths)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

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
        "costguard 2.4.0"
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
            format!("costguard-{subcommand} 2.4.0"),
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
