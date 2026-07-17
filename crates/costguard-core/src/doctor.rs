use crate::config::{ResolvedScanRequest, DEFAULT_MAX_TOTAL_BASE_BYTES};
use crate::scan::scan_with_facts;
use crate::{git, ScanConfig};
use anyhow::Result;
use costguard_cost::CostConfig;
use costguard_dbt::MetadataWarningKind;
use costguard_sql::Platform;
use serde::Serialize;
use serde_yaml::{Mapping, Value};
use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DoctorStatus {
    Pass,
    Warn,
    Fail,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DoctorCheck {
    pub name: String,
    pub status: DoctorStatus,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DoctorReport {
    pub checks: Vec<DoctorCheck>,
}

impl DoctorReport {
    pub fn has_blockers(&self) -> bool {
        self.checks
            .iter()
            .any(|check| check.status == DoctorStatus::Fail)
    }
}

pub fn doctor(config: &ScanConfig, repository_root: &Path) -> Result<DoctorReport> {
    run_doctor(config, repository_root, DEFAULT_MAX_TOTAL_BASE_BYTES)
}

/// Run readiness checks with project-resolved execution limits.
pub fn doctor_resolved(
    request: &ResolvedScanRequest,
    repository_root: &Path,
) -> Result<DoctorReport> {
    run_doctor(
        &request.config,
        repository_root,
        request.max_total_base_bytes,
    )
}

fn run_doctor(
    config: &ScanConfig,
    repository_root: &Path,
    max_total_base_bytes: u64,
) -> Result<DoctorReport> {
    let mut checks = Vec::new();
    checks.push(check(
        "configuration",
        DoctorStatus::Pass,
        "effective configuration loaded and validated",
    ));
    let git_repository = git::is_git_repository(repository_root);
    checks.push(check(
        "git repository",
        if git_repository {
            DoctorStatus::Pass
        } else {
            DoctorStatus::Fail
        },
        if git_repository {
            "repository detected"
        } else {
            "not a git repository; PR comparisons require git history"
        },
    ));
    let committed_head = git_repository && has_committed_head(repository_root);
    checks.push(check(
        "committed HEAD",
        if committed_head {
            DoctorStatus::Pass
        } else {
            DoctorStatus::Fail
        },
        if committed_head {
            "HEAD resolves to a commit"
        } else {
            "no committed HEAD; create an initial commit before running PR checks"
        },
    ));

    let mut scan_config = config.clone();
    scan_config.changed_only = false;
    scan_config.base_branch = None;
    let cost = scan_config.cost.get_or_insert_with(CostConfig::default);
    cost.enabled = true;
    let execution = scan_with_facts(&scan_config, max_total_base_bytes)?;
    let result = execution.result;
    let readiness = execution.readiness;

    let analyzable = readiness.analyzable_files;
    checks.push(check(
        "analyzable files",
        if analyzable > 0 {
            DoctorStatus::Pass
        } else {
            DoctorStatus::Fail
        },
        format!("found {analyzable} SQL, YAML, or Python files"),
    ));
    checks.push(check(
        "analysis policy",
        if result.analysis.passed {
            DoctorStatus::Pass
        } else {
            DoctorStatus::Fail
        },
        if result.analysis.passed {
            "configured analysis policy passed".to_string()
        } else {
            result
                .analysis
                .violations
                .iter()
                .map(|violation| violation.message.as_str())
                .collect::<Vec<_>>()
                .join("; ")
        },
    ));
    let configured_policy_failed = result.should_fail(
        scan_config.fail_on,
        scan_config.min_confidence,
        scan_config
            .cost
            .as_ref()
            .and_then(|cost| cost.fail_on_monthly_delta),
        scan_config
            .cost
            .as_ref()
            .and_then(|cost| cost.fail_on_monthly_delta_gb),
    );
    checks.push(check(
        "configured policy",
        if configured_policy_failed {
            DoctorStatus::Fail
        } else {
            DoctorStatus::Pass
        },
        if configured_policy_failed {
            "the full-project scan violates a configured failure threshold"
        } else {
            "the full-project scan passes configured failure thresholds"
        },
    ));

    let dbt_project = readiness.dbt_project_present;
    checks.push(check(
        "dbt project",
        if dbt_project {
            DoctorStatus::Pass
        } else {
            DoctorStatus::Warn
        },
        if dbt_project {
            "dbt_project.yml detected"
        } else {
            "dbt_project.yml not found; raw SQL scanning remains available"
        },
    ));
    if dbt_project {
        let manifest_degraded = readiness.manifest_path.is_none()
            || readiness.manifest_stale
            || readiness.metadata_warning_kinds.iter().any(|kind| {
                matches!(
                    kind,
                    MetadataWarningKind::NoManifest
                        | MetadataWarningKind::StaleManifest
                        | MetadataWarningKind::DbtProjectParseFailed
                        | MetadataWarningKind::DbtProjectAmbiguousModels
                )
            });
        checks.push(check(
            "dbt manifest",
            if manifest_degraded {
                DoctorStatus::Warn
            } else {
                DoctorStatus::Pass
            },
            if readiness.manifest_path.is_none() {
                "manifest metadata is missing; run dbt compile before CI".to_string()
            } else if readiness.manifest_stale {
                "manifest metadata is stale; run dbt compile before CI".to_string()
            } else if manifest_degraded {
                "manifest metadata loaded with dbt project warnings".to_string()
            } else {
                "manifest metadata is available and current".to_string()
            },
        ));
        if readiness.manifest_path.is_some() {
            let checked = readiness.manifest_checksum_checked;
            let mismatches = readiness.manifest_checksum_mismatches;
            let (status, message) = if checked == 0 {
                (
                    DoctorStatus::Warn,
                    "manifest has no verifiable SHA-256 model checksums".to_string(),
                )
            } else if mismatches == 0 {
                (
                    DoctorStatus::Pass,
                    format!("verified {checked} model source checksum(s)"),
                )
            } else {
                (
                    DoctorStatus::Warn,
                    format!(
                        "{mismatches} of {checked} model source checksum(s) differ; run dbt compile"
                    ),
                )
            };
            checks.push(check("dbt manifest checksum", status, message));
        }
    }

    let parse_failures = readiness.sql_parse_failures
        + readiness.sql_parse_other_failures
        + readiness.sql_parse_compiled_failures;
    checks.push(check(
        "SQL parsing",
        if parse_failures == 0 {
            DoctorStatus::Pass
        } else if result.analysis.passed {
            DoctorStatus::Warn
        } else {
            DoctorStatus::Fail
        },
        format!("{parse_failures} parse failure(s)"),
    ));
    checks.push(check(
        "warehouse",
        if config.platform == Platform::Generic {
            DoctorStatus::Warn
        } else {
            DoctorStatus::Pass
        },
        if config.platform == Platform::Generic {
            "generic warehouse selected; set warehouse in costguard.toml for platform-aware rules"
        } else {
            "warehouse is explicitly configured"
        },
    ));

    let workflow = workflow_contract(repository_root);
    checks.push(check(
        "GitHub workflow",
        if workflow.complete {
            DoctorStatus::Pass
        } else {
            DoctorStatus::Warn
        },
        &workflow.message,
    ));

    if let Some(cost) = result.cost_summary {
        let coverage = &cost.coverage;
        let status =
            if coverage.models_total > 0 && coverage.models_with_usd == coverage.models_total {
                DoctorStatus::Pass
            } else {
                DoctorStatus::Warn
            };
        checks.push(check(
            "cost coverage",
            status,
            format!(
                "{:.0} GB-month relative cost; {:.0}% USD ({}/{} models); {:.0}% mapped spend; grades A/B/C={}/{}/{}",
                cost.project_gb_months,
                coverage.usd_coverage_fraction * 100.0,
                coverage.models_with_usd,
                coverage.models_total,
                coverage.mapped_spend_fraction * 100.0,
                cost.grade_a,
                cost.grade_b,
                cost.grade_c
            ),
        ));
    }

    Ok(DoctorReport { checks })
}

fn check(name: &str, status: DoctorStatus, message: impl Into<String>) -> DoctorCheck {
    DoctorCheck {
        name: name.into(),
        status,
        message: message.into(),
    }
}

fn has_committed_head(root: &Path) -> bool {
    Command::new("git")
        .args(["rev-parse", "--verify", "HEAD"])
        .current_dir(root)
        .output()
        .is_ok_and(|output| output.status.success())
}

struct WorkflowContract {
    complete: bool,
    message: String,
}

fn workflow_contract(root: &Path) -> WorkflowContract {
    let directory = root.join(".github/workflows");
    if !directory.is_dir() {
        return WorkflowContract {
            complete: false,
            message: "no .github/workflows directory; local scans remain available".into(),
        };
    }
    let mut incomplete = None;
    let entries = match std::fs::read_dir(&directory) {
        Ok(entries) => entries,
        Err(error) => {
            return WorkflowContract {
                complete: false,
                message: format!("could not read {}: {error}", directory.display()),
            }
        }
    };
    for entry in entries {
        let path = match entry {
            Ok(entry) => entry.path(),
            Err(error) => {
                incomplete.get_or_insert_with(|| WorkflowContract {
                    complete: false,
                    message: format!("could not inspect a workflow entry: {error}"),
                });
                continue;
            }
        };
        if !matches!(
            path.extension().and_then(|value| value.to_str()),
            Some("yml" | "yaml")
        ) {
            continue;
        }
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) => {
                incomplete.get_or_insert_with(|| WorkflowContract {
                    complete: false,
                    message: format!("could not read {}: {error}", path.display()),
                });
                continue;
            }
        };
        let document: Value = match serde_yaml::from_str(&text) {
            Ok(document) => document,
            Err(error) => {
                incomplete.get_or_insert_with(|| WorkflowContract {
                    complete: false,
                    message: format!("{} is invalid YAML: {error}", path.display()),
                });
                continue;
            }
        };
        match inspect_workflow(&document) {
            Some(missing) if missing.is_empty() => {
                return WorkflowContract {
                    complete: true,
                    message: format!("{} has the recommended PR contract", path.display()),
                }
            }
            Some(missing) => {
                let candidate = WorkflowContract {
                    complete: false,
                    message: format!("{} is missing {}", path.display(), missing.join(", ")),
                };
                if incomplete
                    .as_ref()
                    .is_none_or(|current: &WorkflowContract| {
                        candidate.message.len() < current.message.len()
                    })
                {
                    incomplete = Some(candidate);
                }
            }
            None => {}
        }
    }
    incomplete.unwrap_or_else(|| WorkflowContract {
        complete: false,
        message: "no Costguard GitHub workflow found; local scans remain available".into(),
    })
}

fn inspect_workflow(document: &Value) -> Option<Vec<&'static str>> {
    let root = document.as_mapping()?;
    let jobs = mapping_value(root, "jobs")?.as_mapping()?;
    let top_permissions = mapping_value(root, "permissions");
    let mut best: Option<Vec<&'static str>> = None;
    for job in jobs.values().filter_map(Value::as_mapping) {
        let Some(steps) = mapping_value(job, "steps").and_then(Value::as_sequence) else {
            continue;
        };
        let checkout_complete = steps.iter().filter_map(Value::as_mapping).any(|step| {
            mapping_string(step, "uses").is_some_and(|uses| uses.starts_with("actions/checkout@"))
                && mapping_value(step, "with")
                    .and_then(Value::as_mapping)
                    .and_then(|with| mapping_value(with, "fetch-depth"))
                    .is_some_and(value_is_zero)
        });
        let permissions = mapping_value(job, "permissions").or(top_permissions);
        for step in steps.iter().filter_map(Value::as_mapping) {
            let Some(uses) = mapping_string(step, "uses") else {
                continue;
            };
            if !is_costguard_action(uses) {
                continue;
            }
            let with = mapping_value(step, "with").and_then(Value::as_mapping);
            let missing = [
                (checkout_complete, "full git history"),
                (
                    with.and_then(|value| mapping_value(value, "min-confidence"))
                        .is_some_and(|value| value_is_string(value, "high")),
                    "high confidence gate",
                ),
                (
                    with.and_then(|value| mapping_value(value, "block-only-new"))
                        .is_some_and(value_is_true),
                    "new-finding gate",
                ),
                (
                    with.and_then(|value| mapping_value(value, "pr-comment"))
                        .is_some_and(value_is_true),
                    "sticky PR comment",
                ),
                (
                    with.and_then(|value| mapping_value(value, "github-token"))
                        .and_then(Value::as_str)
                        .is_some_and(|value| !value.trim().is_empty()),
                    "GitHub comment token",
                ),
                (
                    permissions.is_some_and(|value| permission_is(value, "contents", "read")),
                    "contents read permission",
                ),
                (
                    permissions.is_some_and(|value| permission_is(value, "pull-requests", "write")),
                    "pull-request write permission",
                ),
            ]
            .into_iter()
            .filter_map(|(present, label)| (!present).then_some(label))
            .collect::<Vec<_>>();
            if missing.is_empty() {
                return Some(missing);
            }
            if best
                .as_ref()
                .is_none_or(|current| missing.len() < current.len())
            {
                best = Some(missing);
            }
        }
    }
    best
}

fn is_costguard_action(uses: &str) -> bool {
    let action = uses.split_once('@').map_or(uses, |(action, _)| action);
    matches!(
        action,
        "hypertrial/costguard"
            | "hypertrial/costguard/.github/actions/costguard"
            | "./.github/actions/costguard"
    )
}

fn mapping_value<'a>(mapping: &'a Mapping, key: &str) -> Option<&'a Value> {
    mapping.get(Value::String(key.into()))
}

fn mapping_string<'a>(mapping: &'a Mapping, key: &str) -> Option<&'a str> {
    mapping_value(mapping, key).and_then(Value::as_str)
}

fn value_is_zero(value: &Value) -> bool {
    value.as_i64() == Some(0) || value.as_str().is_some_and(|value| value.trim() == "0")
}

fn value_is_true(value: &Value) -> bool {
    value.as_bool() == Some(true)
        || value
            .as_str()
            .is_some_and(|value| value.eq_ignore_ascii_case("true"))
}

fn value_is_string(value: &Value, expected: &str) -> bool {
    value
        .as_str()
        .is_some_and(|value| value.eq_ignore_ascii_case(expected))
}

fn permission_is(value: &Value, key: &str, expected: &str) -> bool {
    value
        .as_mapping()
        .and_then(|permissions| mapping_value(permissions, key))
        .is_some_and(|value| value_is_string(value, expected))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workflow_check_accepts_any_complete_costguard_workflow() {
        let temp = tempfile::tempdir().unwrap();
        let workflows = temp.path().join(".github/workflows");
        std::fs::create_dir_all(&workflows).unwrap();
        std::fs::write(
            workflows.join("a-incomplete.yml"),
            "jobs:\n  unrelated:\n    runs-on: ubuntu-latest\n",
        )
        .unwrap();
        std::fs::write(
            workflows.join("b-complete.yml"),
            r#"permissions:
  contents: read
  pull-requests: write
jobs:
  unrelated:
    runs-on: ubuntu-latest
  costguard:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with:
          fetch-depth: "0"
      - uses: hypertrial/costguard/.github/actions/costguard@v2
        with:
          min-confidence: high
          block-only-new: true
          pr-comment: "true"
          github-token: ${{ github.token }}
"#,
        )
        .unwrap();

        let contract = workflow_contract(temp.path());

        assert!(contract.complete);
        assert!(contract.message.contains("b-complete.yml"));
    }

    #[test]
    fn job_permissions_override_top_level_permissions() {
        let document: Value = serde_yaml::from_str(
            r#"permissions:
  contents: read
  pull-requests: write
jobs:
  costguard:
    permissions:
      pull-requests: write
    steps:
      - uses: actions/checkout@v4
        with: {fetch-depth: 0}
      - uses: ./.github/actions/costguard
        with:
          min-confidence: high
          block-only-new: true
          pr-comment: true
          github-token: token-expression
"#,
        )
        .unwrap();

        let missing = inspect_workflow(&document).unwrap();

        assert_eq!(missing, vec!["contents read permission"]);
    }

    #[test]
    fn comments_and_malformed_yaml_do_not_satisfy_the_workflow_contract() {
        let temp = tempfile::tempdir().unwrap();
        let workflows = temp.path().join(".github/workflows");
        std::fs::create_dir_all(&workflows).unwrap();
        std::fs::write(
            workflows.join("comments.yml"),
            "# uses: hypertrial/costguard@v2\n# fetch-depth: 0\njobs: {}\n",
        )
        .unwrap();
        std::fs::write(workflows.join("malformed.yaml"), "jobs: [\n").unwrap();

        let contract = workflow_contract(temp.path());

        assert!(!contract.complete);
        assert!(
            contract.message.contains("invalid YAML")
                || contract.message.contains("no Costguard GitHub workflow")
        );
    }

    #[test]
    fn scan_facts_detect_changed_manifest_checksum_without_reparsing() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(temp.path().join("models")).unwrap();
        std::fs::create_dir_all(temp.path().join("target")).unwrap();
        std::fs::write(
            temp.path().join("dbt_project.yml"),
            "name: demo\nversion: 1.0.0\nconfig-version: 2\nmodel-paths: [models]\n",
        )
        .unwrap();
        std::fs::write(temp.path().join("models/a.sql"), "select 1\n").unwrap();
        std::fs::write(
            temp.path().join("target/manifest.json"),
            r#"{
  "metadata": {
    "dbt_schema_version": "https://schemas.getdbt.com/dbt/manifest/v12.json",
    "project_name": "demo"
  },
  "nodes": {
    "model.demo.a": {
      "resource_type": "model",
      "name": "a",
      "package_name": "demo",
      "original_file_path": "models/a.sql",
      "path": "a.sql",
      "checksum": {"name": "sha256", "checksum": "not-the-current-checksum"}
    }
  }
}"#,
        )
        .unwrap();
        let config = ScanConfig {
            root: temp.path().to_path_buf(),
            paths: vec![temp.path().to_path_buf()],
            manifest_path: Some("target/manifest.json".into()),
            ..ScanConfig::default()
        };

        let execution = scan_with_facts(&config, DEFAULT_MAX_TOTAL_BASE_BYTES).unwrap();

        assert!(execution.readiness.dbt_project_present);
        assert_eq!(execution.readiness.manifest_checksum_checked, 1);
        assert_eq!(execution.readiness.manifest_checksum_mismatches, 1);
    }
}
