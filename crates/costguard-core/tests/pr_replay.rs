#[path = "common/mod.rs"]
mod common;

use common::{copy_dir_all, fixture, run_git};
use costguard_core::{scan, ScanConfig};
use costguard_diagnostics::Severity;
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
}
