use crate::branch_scan::{
    finding_delta_diagnostics, run_branch_diagnostics, BranchScanInput, BranchScanRun,
};
use crate::config::{RockyConfig, ScanConfig};
use crate::dbt_load::enrich_dbt_project_from_files;
use crate::framework_snapshot::{
    assemble_framework_snapshot, FrameworkSnapshotInput, SnapshotFiles,
};
use crate::git::BaseObjectStore;
use crate::governance::ManagedPolicy;
use crate::owners::OwnerResolver;
use crate::rocky::{
    load_rocky_artifact, verify_rocky_artifact_at_commit_with_store, RockyArtifactEnvelope,
    RockyIntegrityState, RockyProject,
};
use crate::scan_plan::ScanPlan;
use crate::LoadedProject;
use anyhow::{Context, Result};
use costguard_dbt::{
    apply_dbt_project_configs_from_files, parse_dbt_project_with_warnings, parse_manifest_text,
    parse_manifest_with_limit, DbtProject,
};
use costguard_scanner::ProjectFile;
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};

pub(crate) struct BaseBranchScan {
    pub diagnostics: Vec<costguard_diagnostics::Diagnostic>,
    pub cost_summary: Option<costguard_cost::ProjectCostSummary>,
    pub rocky_state: RockyIntegrityState,
    pub rocky_messages: Vec<String>,
    pub rocky_finding_ids: BTreeSet<String>,
}

pub(crate) struct BaseBranchScanInput<'a> {
    pub config: &'a ScanConfig,
    pub rocky_config: &'a RockyConfig,
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
        rocky_config,
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
    let rocky_artifact = load_base_rocky_artifact(root, commit, rocky_config)?;
    let snapshot_spec = base_snapshot_spec(root, plan, config)?;
    let mut requests = snapshot_spec.requests.clone();
    if let Some(artifact) = &rocky_artifact.artifact {
        requests.extend(artifact.inputs.iter().map(|input| {
            (
                input.path.clone(),
                costguard_scanner::effective_max_file_bytes(None),
            )
        }));
    }
    let object_store = BaseObjectStore::load(
        root,
        commit,
        requests,
        snapshot_spec.local_manifest_bytes,
        max_total_base_bytes,
    )?;
    let (base_dbt, base_files) = load_base_snapshot(root, &snapshot_spec, config, &object_store)?;
    let (base_rocky, rocky_state, rocky_messages) =
        finish_base_rocky(commit, rocky_artifact, &object_store);
    let mut base_dbt = base_dbt.unwrap_or_default();
    let rocky_owned_paths = base_rocky
        .as_ref()
        .map(|rocky| {
            rocky
                .source_by_name
                .values()
                .cloned()
                .collect::<BTreeSet<_>>()
        })
        .unwrap_or_else(|| fallback_base_rocky_sql_paths(root, rocky_config, &base_files.context));
    if base_rocky.is_none() {
        base_dbt.models.retain(|_, model| {
            model
                .path
                .as_ref()
                .is_none_or(|path| !rocky_owned_paths.contains(path))
        });
    }
    let mut base_metadata_warnings = Vec::new();
    let dbt_context = base_files
        .context
        .iter()
        .filter(|file| !rocky_owned_paths.contains(&file.root_relative_path))
        .cloned()
        .collect::<Vec<_>>();
    enrich_dbt_project_from_files(&mut base_dbt, &dbt_context, &mut base_metadata_warnings);
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
    let raw_rocky = base_rocky
        .as_ref()
        .map(|rocky| {
            rocky
                .compiled_by_path
                .keys()
                .map(|path| {
                    object_store
                        .text(path)?
                        .with_context(|| {
                            format!("Rocky source {} is missing at base commit", path.display())
                        })
                        .map(|text| (path.clone(), text.to_owned()))
                })
                .collect::<Result<BTreeMap<_, _>>>()
        })
        .transpose()?
        .unwrap_or_default();
    let snapshot = assemble_framework_snapshot(FrameworkSnapshotInput {
        root,
        dbt: base_dbt,
        rocky: base_rocky,
        rocky_owned_paths,
        raw_rocky,
        files: SnapshotFiles {
            context: base_files.context,
            targets: base_files.targets,
        },
        changed_paths: &plan.base_changed_paths,
        pr_mode: true,
        selected_rocky_paths: BTreeSet::new(),
    })?;
    let project = snapshot.project;
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
            analysis_files: &snapshot.analysis_files,
            metadata_diagnostics: Vec::new(),
            write_baseline_path: None,
        }),
    )?;
    let diagnostics = finding_delta_diagnostics(&branch.diagnostics);
    let rocky_finding_ids = diagnostics
        .iter()
        .filter(|diagnostic| is_rocky_diagnostic(&project, diagnostic))
        .map(|diagnostic| diagnostic.governance.finding_id.clone())
        .collect();
    Ok(Some(BaseBranchScan {
        diagnostics,
        cost_summary: branch.cost_analysis.map(|analysis| analysis.summary),
        rocky_state,
        rocky_messages,
        rocky_finding_ids,
    }))
}

fn fallback_base_rocky_sql_paths(
    root: &Path,
    config: &RockyConfig,
    files: &[ProjectFile],
) -> BTreeSet<PathBuf> {
    if !config.configured {
        return BTreeSet::new();
    }
    let models_dir = config
        .models_dir
        .strip_prefix(root)
        .unwrap_or(&config.models_dir);
    files
        .iter()
        .filter(|file| {
            file.root_relative_path.starts_with(models_dir)
                && file
                    .root_relative_path
                    .extension()
                    .and_then(|value| value.to_str())
                    == Some("sql")
        })
        .map(|file| file.root_relative_path.clone())
        .collect()
}

fn is_rocky_diagnostic(
    project: &LoadedProject,
    diagnostic: &costguard_diagnostics::Diagnostic,
) -> bool {
    project
        .graph
        .model_for_path(&diagnostic.path)
        .is_some_and(|model| model.framework == costguard_project::Framework::Rocky)
        || project.rocky_owned_paths.contains(&diagnostic.path)
}

struct BaseRockyArtifactLoad {
    artifact: Option<RockyArtifactEnvelope>,
    state: RockyIntegrityState,
    messages: Vec<String>,
}

fn load_base_rocky_artifact(
    root: &Path,
    commit: &str,
    config: &RockyConfig,
) -> Result<BaseRockyArtifactLoad> {
    if !config.configured {
        return Ok(BaseRockyArtifactLoad {
            artifact: None,
            state: RockyIntegrityState::NotConfigured,
            messages: Vec::new(),
        });
    }
    let Some(path) = &config.base_artifact_path else {
        return Ok(BaseRockyArtifactLoad {
            artifact: None,
            state: RockyIntegrityState::Missing,
            messages: vec!["no base Rocky artifact was supplied".into()],
        });
    };
    let path = if path.is_absolute() {
        path.clone()
    } else {
        root.join(path)
    };
    if !path.is_file() {
        anyhow::bail!(
            "base Rocky artifact path does not exist: {}",
            path.display()
        );
    }
    match load_rocky_artifact(&path, config.max_artifact_bytes) {
        Ok(artifact) if artifact.git_commit == commit => Ok(BaseRockyArtifactLoad {
            artifact: Some(artifact),
            state: RockyIntegrityState::Verified,
            messages: Vec::new(),
        }),
        Ok(artifact) => Ok(BaseRockyArtifactLoad {
            artifact: None,
            state: RockyIntegrityState::Invalid,
            messages: vec![format!(
                "base Rocky artifact is unusable: base Rocky artifact commit {} does not match comparison commit {}",
                artifact.git_commit, commit
            )],
        }),
        Err(error) => Ok(BaseRockyArtifactLoad {
            artifact: None,
            state: RockyIntegrityState::Invalid,
            messages: vec![format!("base Rocky artifact is unusable: {error:#}")],
        }),
    }
}

fn finish_base_rocky(
    commit: &str,
    load: BaseRockyArtifactLoad,
    contents: &BaseObjectStore,
) -> (Option<RockyProject>, RockyIntegrityState, Vec<String>) {
    let Some(artifact) = load.artifact else {
        return (None, load.state, load.messages);
    };
    match verify_rocky_artifact_at_commit_with_store(commit, &artifact, contents) {
        Ok(project) => {
            let messages = project.unresolved_dependencies.clone();
            (Some(project), RockyIntegrityState::Verified, messages)
        }
        Err(error) => (
            None,
            RockyIntegrityState::Invalid,
            vec![format!("base Rocky artifact is unusable: {error:#}")],
        ),
    }
}

struct BaseSnapshotSpec {
    rel_paths: BTreeSet<PathBuf>,
    target_paths: HashSet<PathBuf>,
    explicit_manifest: Option<PathBuf>,
    git_manifest: Option<PathBuf>,
    local_manifest_bytes: u64,
    requests: Vec<(PathBuf, u64)>,
}

fn base_snapshot_spec(
    root: &Path,
    plan: &ScanPlan,
    config: &ScanConfig,
) -> Result<BaseSnapshotSpec> {
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
        observed
    } else {
        0
    };
    Ok(BaseSnapshotSpec {
        rel_paths,
        target_paths: plan.base_changed_paths.clone(),
        explicit_manifest,
        git_manifest,
        local_manifest_bytes,
        requests,
    })
}

fn load_base_snapshot(
    root: &Path,
    spec: &BaseSnapshotSpec,
    config: &ScanConfig,
    contents: &BaseObjectStore,
) -> Result<(Option<DbtProject>, BaseBranchFiles)> {
    let base_dbt = match (&spec.explicit_manifest, &spec.git_manifest) {
        (Some(path), _) => Some(parse_manifest_with_limit(path, config.max_manifest_bytes)?),
        (None, Some(path)) => contents.text(path)?.map(parse_manifest_text).transpose()?,
        (None, None) => None,
    };

    let mut context = Vec::new();
    for rel_path in &spec.rel_paths {
        let Some(text) = contents.text(rel_path)? else {
            continue;
        };
        let path = root.join(rel_path);
        let text = text.to_owned();
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
        .filter(|file| spec.target_paths.contains(&file.root_relative_path))
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
