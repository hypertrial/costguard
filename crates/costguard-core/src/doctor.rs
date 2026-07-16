use crate::{git, scan, ScanConfig};
use anyhow::{Context, Result};
use costguard_cost::CostConfig;
use costguard_sql::Platform;
use serde::Serialize;
use std::path::{Path, PathBuf};
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
    let result = scan(&scan_config)?;

    let analyzable = result.counts.sql + result.counts.yaml + result.counts.python;
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

    let dbt_project = config.root.join("dbt_project.yml").is_file();
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
        let metadata_rules: Vec<&str> = result
            .diagnostics
            .iter()
            .map(|diagnostic| diagnostic.rule_id.as_str())
            .filter(|rule| matches!(*rule, "SQLCOST023" | "SQLCOST045" | "SQLCOST046"))
            .collect();
        checks.push(check(
            "dbt manifest",
            if metadata_rules.is_empty() {
                DoctorStatus::Pass
            } else {
                DoctorStatus::Warn
            },
            if metadata_rules.is_empty() {
                "manifest metadata is available and current".to_string()
            } else {
                format!(
                    "metadata warning(s): {}; run dbt compile before CI",
                    metadata_rules.join(", ")
                )
            },
        ));
        if let Some((checked, mismatches)) = manifest_checksum_counts(config)? {
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

    let parse_failures = result.metrics.sql_parse_failures
        + result.metrics.sql_parse_other_failures
        + result.metrics.sql_parse_compiled_failures;
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

    let workflow = workflow_contract(repository_root)?;
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

fn manifest_checksum_counts(config: &ScanConfig) -> Result<Option<(usize, usize)>> {
    let manifest_path = config
        .manifest_path
        .as_ref()
        .map(|path| resolve_path(&config.root, path))
        .unwrap_or_else(|| config.root.join("target/manifest.json"));
    if !manifest_path.is_file() {
        return Ok(None);
    }
    let project =
        costguard_dbt::parse_manifest_with_limit(&manifest_path, config.max_manifest_bytes)?;
    let mut checked = 0usize;
    let mut mismatches = 0usize;
    for model in project.models.values() {
        let Some(expected) = model.checksum.as_deref() else {
            continue;
        };
        if !model
            .checksum_kind
            .as_deref()
            .is_some_and(|kind| kind.eq_ignore_ascii_case("sha256"))
        {
            continue;
        }
        let Some(path) = model.path.as_deref() else {
            continue;
        };
        checked += 1;
        let source_path = resolve_path(&config.root, path);
        let matches = std::fs::read_to_string(&source_path)
            .map(|source| costguard_diagnostics::hex_sha256(source.as_bytes()) == expected)
            .unwrap_or(false);
        if !matches {
            mismatches += 1;
        }
    }
    Ok(Some((checked, mismatches)))
}

fn resolve_path(root: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    }
}

struct WorkflowContract {
    complete: bool,
    message: String,
}

fn workflow_contract(root: &Path) -> Result<WorkflowContract> {
    let directory = root.join(".github/workflows");
    if !directory.is_dir() {
        return Ok(WorkflowContract {
            complete: false,
            message: "no .github/workflows directory; local scans remain available".into(),
        });
    }
    let mut incomplete = None;
    for entry in std::fs::read_dir(&directory)
        .with_context(|| format!("failed to read {}", directory.display()))?
    {
        let path = entry?.path();
        if !matches!(
            path.extension().and_then(|value| value.to_str()),
            Some("yml" | "yaml")
        ) {
            continue;
        }
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        if !text.contains("hypertrial/costguard") && !text.contains("./.github/actions/costguard") {
            continue;
        }
        let token_configured = text.lines().any(|line| {
            line.trim()
                .strip_prefix("github-token:")
                .is_some_and(|value| !value.trim().is_empty())
        });
        let missing: Vec<&str> = [
            (text.contains("fetch-depth: 0"), "full git history"),
            (
                text.contains("min-confidence: high"),
                "high confidence gate",
            ),
            (text.contains("block-only-new: true"), "new-finding gate"),
            (text.contains("pr-comment: true"), "sticky PR comment"),
            (token_configured, "GitHub comment token"),
            (
                text.contains("pull-requests: write"),
                "pull-request write permission",
            ),
        ]
        .into_iter()
        .filter_map(|(present, label)| (!present).then_some(label))
        .collect();
        if missing.is_empty() {
            return Ok(WorkflowContract {
                complete: true,
                message: format!("{} has the recommended PR contract", path.display()),
            });
        }
        incomplete.get_or_insert_with(|| WorkflowContract {
            complete: false,
            message: format!("{} is missing {}", path.display(), missing.join(", ")),
        });
    }
    Ok(incomplete.unwrap_or_else(|| WorkflowContract {
        complete: false,
        message: "no Costguard GitHub workflow found; local scans remain available".into(),
    }))
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
            "uses: hypertrial/costguard/.github/actions/costguard@v2\n",
        )
        .unwrap();
        std::fs::write(
            workflows.join("b-complete.yml"),
            r#"uses: hypertrial/costguard/.github/actions/costguard@v2
fetch-depth: 0
min-confidence: high
block-only-new: true
pr-comment: true
github-token: ${{ github.token }}
pull-requests: write
"#,
        )
        .unwrap();

        let contract = workflow_contract(temp.path()).unwrap();

        assert!(contract.complete);
        assert!(contract.message.contains("b-complete.yml"));
    }

    #[test]
    fn manifest_checksum_check_detects_changed_model_source() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(temp.path().join("models")).unwrap();
        std::fs::create_dir_all(temp.path().join("target")).unwrap();
        std::fs::write(temp.path().join("models/a.sql"), "select 1\n").unwrap();
        std::fs::write(
            temp.path().join("target/manifest.json"),
            r#"{
  "nodes": {
    "model.demo.a": {
      "resource_type": "model",
      "name": "a",
      "original_file_path": "models/a.sql",
      "checksum": {"name": "sha256", "checksum": "not-the-current-checksum"}
    }
  }
}"#,
        )
        .unwrap();
        let config = ScanConfig {
            root: temp.path().to_path_buf(),
            ..ScanConfig::default()
        };

        assert_eq!(manifest_checksum_counts(&config).unwrap(), Some((1, 1)));
    }
}
