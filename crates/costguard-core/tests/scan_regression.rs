#[path = "common/mod.rs"]
mod common;

use common::run_git;
use costguard_core::{scan, ParseInput, Platform, ScanConfig, ScanResult};
use costguard_diagnostics::Severity;
use serde_json::json;
use std::path::PathBuf;

#[test]
fn full_and_pr_scans_preserve_results_and_compiled_selection() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    std::fs::create_dir_all(root.join("models/marts")).expect("create models");
    std::fs::create_dir_all(root.join("target")).expect("create target");

    for name in ["alpha", "beta"] {
        std::fs::write(
            root.join(format!("models/marts/{name}.sql")),
            "select 1 as id\n",
        )
        .expect("write initial model");
    }
    let nodes = ["alpha", "beta"]
        .into_iter()
        .map(|name| {
            (
                format!("model.test.{name}"),
                json!({
                    "resource_type": "model",
                    "name": name,
                    "original_file_path": format!("models/marts/{name}.sql"),
                    "config": {"materialized": "view"},
                    "depends_on": {"nodes": []},
                    "compiled_code": format!("select * from raw.{name}"),
                }),
            )
        })
        .collect::<serde_json::Map<_, _>>();
    std::fs::write(
        root.join("target/manifest.json"),
        serde_json::to_string(&json!({"nodes": nodes})).expect("serialize manifest"),
    )
    .expect("write manifest");

    run_git(root, &["init"]);
    run_git(root, &["checkout", "-b", "main"]);
    run_git(root, &["config", "user.email", "costguard@example.com"]);
    run_git(root, &["config", "user.name", "Costguard Test"]);
    run_git(root, &["add", "."]);
    run_git(root, &["commit", "-m", "initial"]);
    run_git(root, &["checkout", "-b", "feature"]);

    let raw = "{% set cols = ['id'] %} select {{ cols | join(', ') }} from raw.events\n";
    for name in ["alpha", "beta"] {
        std::fs::write(root.join(format!("models/marts/{name}.sql")), raw).expect("modify model");
    }

    let base = ScanConfig {
        root: root.to_path_buf(),
        paths: vec![PathBuf::from("models")],
        platform: Platform::Trino,
        manifest_path: Some(root.join("target/manifest.json")),
        fail_on: Some(Severity::Critical),
        ..ScanConfig::default()
    };
    let full = scan(&base).expect("full scan");
    let repeated = scan(&base).expect("repeated full scan");
    let pr = scan(&ScanConfig {
        changed_only: true,
        base_branch: Some("main".into()),
        ..base
    })
    .expect("pr scan");

    assert_eq!(diagnostic_signature(&full), diagnostic_signature(&repeated));
    assert_eq!(diagnostic_signature(&full), diagnostic_signature(&pr));
    assert_eq!(
        serde_json::to_value(&full.metrics).expect("full metrics"),
        serde_json::to_value(&pr.metrics).expect("pr metrics")
    );
    assert_eq!(
        serde_json::to_value(&full.file_parse_status).expect("full parse status"),
        serde_json::to_value(&pr.file_parse_status).expect("pr parse status")
    );
    assert!(full
        .file_parse_status
        .iter()
        .all(|status| status.parse_input == ParseInput::Compiled));
    assert!(full.diagnostics.windows(2).all(|pair| {
        (&pair[0].path, pair[0].line, &pair[0].rule_id)
            <= (&pair[1].path, pair[1].line, &pair[1].rule_id)
    }));
}

fn diagnostic_signature(result: &ScanResult) -> Vec<(PathBuf, usize, usize, String, String)> {
    result
        .diagnostics
        .iter()
        .map(|diagnostic| {
            (
                diagnostic.path.clone(),
                diagnostic.line,
                diagnostic.column,
                diagnostic.rule_id.clone(),
                diagnostic.message.clone(),
            )
        })
        .collect()
}
