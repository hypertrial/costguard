use crate::branch_scan::{
    finding_delta_diagnostics, run_branch_diagnostics, BranchScanInput, BranchScanRun,
};
use crate::config::ScanConfig;
use crate::dbt_load::enrich_dbt_project_from_files;
use crate::governance::ManagedPolicy;
use crate::owners::OwnerResolver;
use crate::scan_plan::ScanPlan;
use crate::Project;
use anyhow::{Context, Result};
use costguard_dbt::{
    apply_dbt_project_configs_from_files, parse_dbt_project_with_warnings, parse_manifest_text,
    parse_manifest_with_limit, DbtProject,
};
use costguard_scanner::ProjectFile;
use std::path::{Path, PathBuf};

pub(crate) struct BaseBranchScan {
    pub diagnostics: Vec<costguard_diagnostics::Diagnostic>,
    pub cost_summary: Option<costguard_cost::ProjectCostSummary>,
}

pub(crate) struct BaseBranchScanInput<'a> {
    pub config: &'a ScanConfig,
    pub root: &'a Path,
    pub plan: &'a ScanPlan,
    pub owner_resolver: &'a OwnerResolver,
    pub managed_policy: Option<&'a ManagedPolicy>,
    pub started_at: chrono::DateTime<chrono::Utc>,
    pub baseline: Option<&'a crate::FindingBaseline>,
    pub policy_digest: &'a str,
    pub max_total_base_bytes: u64,
}

pub(crate) fn run_base_branch_scan(
    input: BaseBranchScanInput<'_>,
) -> Result<Option<BaseBranchScan>> {
    let BaseBranchScanInput {
        config,
        root,
        plan,
        owner_resolver,
        managed_policy,
        started_at,
        baseline,
        policy_digest,
        max_total_base_bytes,
    } = input;
    if !plan.pr_mode {
        return Ok(None);
    }
    let commit = plan
        .base_commit
        .as_deref()
        .context("PR scan plan is missing its resolved base commit")?;
    let (base_dbt, base_files) =
        load_base_snapshot(root, commit, plan, config, max_total_base_bytes)?;
    let mut base_dbt = base_dbt.unwrap_or_default();
    let mut base_metadata_warnings = Vec::new();
    enrich_dbt_project_from_files(
        &mut base_dbt,
        &base_files.context,
        &mut base_metadata_warnings,
    );
    let base_project_files = base_files
        .context
        .iter()
        .filter(|file| {
            file.path
                .file_name()
                .is_some_and(|name| name == "dbt_project.yml")
        })
        .map(|file| {
            let (project, warnings) = parse_dbt_project_with_warnings(&file.text, &file.path);
            base_metadata_warnings.extend(warnings);
            project
        })
        .collect::<Vec<_>>();
    apply_dbt_project_configs_from_files(root, &base_project_files, &mut base_dbt);
    let base_dbt = (!base_dbt.models.is_empty()).then_some(base_dbt);
    let project = Project {
        root: root.to_path_buf(),
        files: base_files.targets,
        dbt: base_dbt,
    };
    let branch = run_branch_diagnostics(
        BranchScanInput {
            config,
            root,
            owner_resolver,
            managed_policy,
            started_at,
            baseline,
            policy_digest,
        }
        .with_scan(BranchScanRun {
            project: &project,
            analysis_files: &base_files.context,
            metadata_diagnostics: Vec::new(),
            write_baseline_path: None,
        }),
    )?;
    Ok(Some(BaseBranchScan {
        diagnostics: finding_delta_diagnostics(&branch.diagnostics),
        cost_summary: branch.cost_analysis.map(|analysis| analysis.summary),
    }))
}

fn load_base_snapshot(
    root: &Path,
    commit: &str,
    plan: &ScanPlan,
    config: &ScanConfig,
    max_total_base_bytes: u64,
) -> Result<(Option<DbtProject>, BaseBranchFiles)> {
    let rel_paths = base_context_rel_paths(root, plan, config);
    let max_file_bytes = costguard_scanner::effective_max_file_bytes(config.max_file_bytes);
    let mut requests = rel_paths
        .iter()
        .cloned()
        .map(|path| (path, max_file_bytes))
        .collect::<Vec<_>>();

    let explicit_manifest = config.base_manifest_path.as_ref().map(|path| {
        if path.is_absolute() {
            path.clone()
        } else {
            root.join(path)
        }
    });
    let git_manifest = explicit_manifest
        .is_none()
        .then(|| base_manifest_rel_path(root, config));
    if let Some(path) = &git_manifest {
        requests.push((path.clone(), config.max_manifest_bytes));
    }

    let local_manifest_bytes = if let Some(path) = &explicit_manifest {
        let observed = std::fs::metadata(path)
            .with_context(|| format!("failed to inspect manifest {}", path.display()))?
            .len();
        if observed > config.max_manifest_bytes {
            anyhow::bail!(
                "manifest {} is {} bytes, exceeding configured limit of {} bytes",
                path.display(),
                observed,
                config.max_manifest_bytes
            );
        }
        if observed > max_total_base_bytes {
            anyhow::bail!(
                "base snapshot is {} bytes, exceeding configured aggregate limit of {} bytes",
                observed,
                max_total_base_bytes
            );
        }
        observed
    } else {
        0
    };
    let mut contents = crate::git::files_at_commit_with_budget(
        root,
        commit,
        &requests,
        local_manifest_bytes,
        max_total_base_bytes,
    )?;

    let base_dbt = match (explicit_manifest, git_manifest) {
        (Some(path), _) => Some(parse_manifest_with_limit(&path, config.max_manifest_bytes)?),
        (None, Some(path)) => contents
            .remove(&path)
            .flatten()
            .map(|text| parse_manifest_text(&text))
            .transpose()?,
        (None, None) => None,
    };

    let mut context = Vec::new();
    for rel_path in rel_paths {
        let Some(text) = contents.remove(&rel_path).flatten() else {
            continue;
        };
        let path = root.join(&rel_path);
        context.push(ProjectFile {
            kind: costguard_scanner::classify(&path, root),
            path,
            root_relative_path: rel_path.clone(),
            line_index: costguard_diagnostics::LineIndex::new(&text),
            text,
        });
    }
    context.sort_by(|left, right| left.path.cmp(&right.path));
    let targets = context
        .iter()
        .filter(|file| plan.base_changed_paths.contains(&file.root_relative_path))
        .cloned()
        .collect();
    Ok((base_dbt, BaseBranchFiles { targets, context }))
}

fn base_manifest_rel_path(root: &Path, config: &ScanConfig) -> PathBuf {
    let resolved = config
        .manifest_path
        .as_ref()
        .map(|path| {
            if path.is_absolute() {
                path.clone()
            } else {
                root.join(path)
            }
        })
        .unwrap_or_else(|| root.join("target/manifest.json"));
    resolved
        .strip_prefix(root)
        .map(Path::to_path_buf)
        .unwrap_or_else(|_| PathBuf::from("target/manifest.json"))
}

struct BaseBranchFiles {
    targets: Vec<ProjectFile>,
    context: Vec<ProjectFile>,
}

fn base_context_rel_paths(
    root: &Path,
    plan: &ScanPlan,
    config: &ScanConfig,
) -> std::collections::BTreeSet<PathBuf> {
    let mut rel_paths = plan
        .context
        .iter()
        .map(|file| file.root_relative_path.clone())
        .chain(
            plan.base_changed_paths
                .iter()
                .filter(|path| !base_path_is_ignored(root, path, &config.ignore))
                .cloned(),
        )
        .collect::<std::collections::BTreeSet<_>>();
    for scan_root in crate::dbt_load::scan_roots(root, &config.paths) {
        let project_file = scan_root.join("dbt_project.yml");
        if let Ok(relative) = project_file.strip_prefix(root) {
            rel_paths.insert(relative.to_path_buf());
        }
    }

    rel_paths.retain(|rel_path| {
        matches!(
            costguard_scanner::classify(&root.join(rel_path), root),
            costguard_scanner::FileKind::Sql
                | costguard_scanner::FileKind::DbtSqlModel
                | costguard_scanner::FileKind::DbtYaml
                | costguard_scanner::FileKind::Python
        )
    });
    rel_paths
}

fn base_path_is_ignored(root: &Path, path: &Path, ignored: &[PathBuf]) -> bool {
    ignored.iter().any(|ignored| {
        let relative = ignored.strip_prefix(root).unwrap_or(ignored);
        path == relative || path.starts_with(relative)
    })
}
