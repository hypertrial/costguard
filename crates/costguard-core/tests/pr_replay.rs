#[path = "common/mod.rs"]
mod common;

use common::{copy_dir_all, fixture, run_git};
use costguard_core::{scan, ScanConfig, Waiver};
use costguard_diagnostics::Severity;
use costguard_rules::RuleOverride;
use std::fs;
use std::path::PathBuf;

#[test]
fn pr_mode_scopes_changed_files_and_emits_incremental_rule() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let root = tempdir.path().to_path_buf();
    let source = fixture("incremental_missing");

    copy_dir_all(&source, &root).expect("copy fixture");
    run_git(&root, &["init"]);
    run_git(&root, &["config", "user.email", "costguard@test"]);
    run_git(&root, &["config", "user.name", "Costguard"]);
    run_git(&root, &["add", "."]);
    run_git(&root, &["commit", "-m", "initial"]);

    let model_path = root.join("models/marts/fct_events.sql");
    let mut sql = fs::read_to_string(&model_path).expect("read model");
    sql.push_str("\n-- costguard pr replay edit\n");
    fs::write(&model_path, sql).expect("write model");
    run_git(&root, &["add", "."]);
    run_git(&root, &["commit", "-m", "edit incremental model"]);

    let config = ScanConfig {
        root: root.clone(),
        changed_only: true,
        base_branch: Some("HEAD~1".into()),
        paths: vec![PathBuf::from("models")],
        fail_on: Some(Severity::High),
        ..ScanConfig::default()
    };
    let result = scan(&config).expect("scan pr mode");

    let summary = result
        .pr_summary
        .as_ref()
        .expect("pr summary should be present");
    assert_eq!(summary.changed_files.len(), 1);
    assert!(summary.changed_files[0]
        .to_string_lossy()
        .contains("fct_events.sql"));
    assert!(
        result
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.rule_id == "SQLCOST005"),
        "expected SQLCOST005 on edited incremental model"
    );
    let delta = summary.finding_delta.as_ref().expect("finding delta");
    assert!(
        delta.unchanged >= 1,
        "metadata-backed finding should compare on both branches"
    );
    assert_eq!(delta.introduced, 0);
    assert_eq!(delta.resolved, 0);
}

fn init_repo(root: &std::path::Path) {
    run_git(root, &["init"]);
    run_git(root, &["config", "user.email", "costguard@test"]);
    run_git(root, &["config", "user.name", "Costguard"]);
    run_git(root, &["add", "."]);
    run_git(root, &["commit", "-m", "initial"]);
}

fn pr_config(root: &std::path::Path) -> ScanConfig {
    ScanConfig {
        root: root.to_path_buf(),
        changed_only: true,
        base_branch: Some("HEAD".into()),
        paths: vec![PathBuf::from("models")],
        ..ScanConfig::default()
    }
}

#[test]
fn unchanged_context_findings_are_not_reported_as_resolved() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let root = tempdir.path();
    fs::create_dir_all(root.join("models")).expect("models");
    fs::write(
        root.join("models/unchanged.sql"),
        "select * from raw.events\n",
    )
    .expect("unchanged model");
    fs::write(root.join("models/changed.sql"), "select 1\n").expect("changed model");
    init_repo(root);
    fs::write(root.join("models/changed.sql"), "select 1\n-- changed\n").expect("edit");

    let result = scan(&pr_config(root)).expect("scan");
    let delta = result.pr_summary.unwrap().finding_delta.unwrap();
    assert_eq!(delta.resolved, 0);
    assert_eq!(delta.introduced, 0);
}

#[test]
fn python_findings_are_compared_across_branches() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let root = tempdir.path();
    fs::create_dir_all(root.join("models")).expect("models");
    fs::write(
        root.join("models/process.py"),
        "for _, row in df.iterrows():\n    pass\n",
    )
    .expect("python model");
    init_repo(root);
    fs::write(
        root.join("models/process.py"),
        "result = df.with_column('x', df['a'] + df['b'])\n",
    )
    .expect("fix python model");

    let result = scan(&pr_config(root)).expect("scan");
    let delta = result.pr_summary.unwrap().finding_delta.unwrap();
    assert_eq!(delta.resolved, 1);
}

#[test]
fn inline_suppressions_and_waivers_are_excluded_symmetrically() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let root = tempdir.path();
    fs::create_dir_all(root.join("models")).expect("models");
    fs::write(
        root.join("models/suppressed.sql"),
        "-- costguard: disable-file=SQLCOST001\nselect * from raw.events\n",
    )
    .expect("suppressed model");
    fs::write(root.join("models/waived.sql"), "select * from raw.events\n").expect("waived model");
    init_repo(root);
    fs::write(
        root.join("models/suppressed.sql"),
        "-- costguard: disable-file=SQLCOST001\nselect * from raw.events\n-- changed\n",
    )
    .expect("edit suppressed");
    fs::write(
        root.join("models/waived.sql"),
        "select * from raw.events\n-- changed\n",
    )
    .expect("edit waived");
    let mut config = pr_config(root);
    config.waivers.push(Waiver {
        id: "CG-1".into(),
        finding_id: None,
        rule_id: Some("SQLCOST001".into()),
        path: "models/waived.sql".into(),
        owner: "@data".into(),
        reason: "approved migration".into(),
        ticket_url: "https://example.com/CG-1".into(),
        approver: "@lead".into(),
        created_at: "2026-01-01T00:00:00Z".into(),
        expires_at: "2099-01-01T00:00:00Z".into(),
    });

    let result = scan(&config).expect("scan");
    let delta = result.pr_summary.unwrap().finding_delta.unwrap();
    assert_eq!(delta.introduced, 0);
    assert_eq!(delta.regressed, 0);
    assert_eq!(delta.resolved, 0);
    assert_eq!(delta.unchanged, 0);
}

#[test]
fn baselines_apply_to_both_branches() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let root = tempdir.path();
    fs::create_dir_all(root.join("models")).expect("models");
    fs::write(root.join("models/risky.sql"), "select * from raw.events\n").expect("model");
    init_repo(root);

    let baseline_path = root.join("costguard-baseline.json");
    scan(&ScanConfig {
        root: root.to_path_buf(),
        paths: vec![PathBuf::from("models")],
        write_baseline_path: Some(baseline_path.clone()),
        ..ScanConfig::default()
    })
    .expect("write baseline");
    fs::write(
        root.join("models/risky.sql"),
        "select * from raw.events\n-- changed\n",
    )
    .expect("edit model");

    let mut config = pr_config(root);
    config.baseline_path = Some(baseline_path);
    let result = scan(&config).expect("scan");
    let delta = result.pr_summary.unwrap().finding_delta.unwrap();
    assert_eq!(delta.introduced, 0);
    assert_eq!(delta.regressed, 0);
    assert_eq!(delta.resolved, 0);
    assert_eq!(delta.unchanged, 0);
}

#[test]
fn rule_disables_apply_to_both_branches() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let root = tempdir.path();
    fs::create_dir_all(root.join("models")).expect("models");
    fs::write(root.join("models/risky.sql"), "select * from raw.events\n").expect("model");
    init_repo(root);
    fs::write(
        root.join("models/risky.sql"),
        "select * from raw.events\n-- changed\n",
    )
    .expect("edit model");

    let mut config = pr_config(root);
    for rule_id in ["SQLCOST001", "SQLCOST002"] {
        config.rule_overrides.insert(
            rule_id.into(),
            RuleOverride {
                enabled: Some(false),
                ..RuleOverride::default()
            },
        );
    }
    let result = scan(&config).expect("scan");
    let delta = result.pr_summary.unwrap().finding_delta.unwrap();
    assert_eq!(delta.introduced, 0);
    assert_eq!(delta.regressed, 0);
    assert_eq!(delta.resolved, 0);
}

#[test]
fn oversized_base_source_fails_closed() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let root = tempdir.path();
    fs::create_dir_all(root.join("models")).expect("models");
    fs::write(root.join("models/deleted.sql"), "12345").expect("model");
    init_repo(root);
    fs::remove_file(root.join("models/deleted.sql")).expect("delete model");
    let mut config = pr_config(root);
    config.max_file_bytes = Some(4);
    let error = scan(&config).expect_err("oversized base source");
    let message = error.to_string();
    assert!(message.contains("models/deleted.sql"));
    assert!(message.contains("5 bytes"));
    assert!(message.contains("4 bytes"));
}

#[test]
fn oversized_git_base_manifest_fails_closed() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let root = tempdir.path();
    fs::create_dir_all(root.join("models")).expect("models");
    fs::create_dir_all(root.join("target")).expect("target");
    fs::write(root.join("models/model.sql"), "select 1\n").expect("model");
    fs::write(root.join("target/manifest.json"), "{\"nodes\": {}}\n").expect("manifest");
    init_repo(root);
    fs::write(root.join("models/model.sql"), "select 2\n").expect("edit model");
    fs::write(root.join("target/manifest.json"), "{}").expect("small head manifest");
    let mut config = pr_config(root);
    config.max_manifest_bytes = 2;
    let error = scan(&config).expect_err("oversized base manifest");
    let message = error.to_string();
    assert!(message.contains("target/manifest.json"));
    assert!(message.contains("2 bytes"));
}
