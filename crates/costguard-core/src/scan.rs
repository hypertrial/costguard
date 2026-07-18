use crate::analysis_report::{collect_analysis_violations, AnalysisViolationInput};
use crate::base_replay::{run_base_branch_scan, BaseBranchScan, BaseBranchScanInput};
use crate::baseline::{
    load_finding_baseline, validate_baseline_policy_digest, validate_finding_baseline,
};
use crate::branch_scan::{
    finding_delta_diagnostics, run_branch_diagnostics, BranchScanInput, BranchScanRun,
};
use crate::config::{ResolvedScanRequest, RockyConfig, ScanConfig, DEFAULT_MAX_TOTAL_BASE_BYTES};
use crate::dbt_graph::enrich_pr_summary;
use crate::dbt_load::{detect_checksum_mismatches, load_dbt_project, manifest_checksum_counts};
use crate::gates::evaluate_gates;
use crate::governance::{load_managed_policy, validate_local_policy_controls, ManagedPolicy};
use crate::metadata_report::{
    build_context_report, metadata_diagnostics, partition_metadata_warnings, skipped_files_all,
};
use crate::owners::OwnerResolver;
use crate::rocky::{
    load_rocky_artifact, verify_rocky_artifact_current, RockyIntegrity, RockyIntegrityState,
    RockyProject,
};
use crate::scan_plan::{build_scan_plan, ScanPlan};
use crate::sql_analysis::{build_file_parse_status, build_scan_metrics};
use crate::state_diff::classify_findings;
use crate::{
    EnforcementPreview, LoadedProject, ManifestIntegrity, PolicyMetadata, PrSummary, Project,
    RunMetadata, ScanResult,
};
use anyhow::{Context, Result};
use costguard_cost::{compute_blast_radius, compute_pr_impact};
use costguard_dbt::{MetadataWarning, MetadataWarningKind};
use costguard_scanner::{DiscoveryOptions, ScanCounts};
use std::path::{Path, PathBuf};

/// Run a full project scan and return diagnostics, metrics, and optional cost summary.
pub fn scan(config: &ScanConfig) -> Result<ScanResult> {
    crate::validate_scan_config(config)?;
    Ok(run_scan(
        config,
        &RockyConfig::default(),
        DEFAULT_MAX_TOTAL_BASE_BYTES,
    )?
    .result)
}

/// Run a scan with project-resolved execution safety limits.
pub fn scan_resolved(request: &ResolvedScanRequest) -> Result<ScanResult> {
    crate::validate_resolved_scan_request(request)?;
    let max_total_base_bytes = if request.max_total_base_bytes == 0 {
        DEFAULT_MAX_TOTAL_BASE_BYTES
    } else {
        request.max_total_base_bytes
    };
    Ok(run_scan(&request.config, &request.rocky, max_total_base_bytes)?.result)
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ReadinessFacts {
    pub analyzable_files: usize,
    pub sql_parse_failures: usize,
    pub sql_parse_other_failures: usize,
    pub sql_parse_compiled_failures: usize,
    pub dbt_project_present: bool,
    pub manifest_path: Option<PathBuf>,
    pub manifest_stale: bool,
    pub manifest_checksum_checked: usize,
    pub manifest_checksum_mismatches: usize,
    pub metadata_warning_kinds: Vec<MetadataWarningKind>,
    pub rocky_configured: bool,
    pub rocky_artifact_verified: bool,
    pub rocky_messages: Vec<String>,
}

pub(crate) struct ScanExecution {
    pub result: ScanResult,
    pub readiness: ReadinessFacts,
}

pub(crate) fn scan_with_facts(
    config: &ScanConfig,
    max_total_base_bytes: u64,
) -> Result<ScanExecution> {
    crate::validate_scan_config(config)?;
    run_scan(
        config,
        &RockyConfig::default(),
        if max_total_base_bytes == 0 {
            DEFAULT_MAX_TOTAL_BASE_BYTES
        } else {
            max_total_base_bytes
        },
    )
}

pub(crate) fn scan_with_rocky_facts(
    config: &ScanConfig,
    rocky: &RockyConfig,
    max_total_base_bytes: u64,
) -> Result<ScanExecution> {
    crate::validate_scan_config(config)?;
    run_scan(
        config,
        rocky,
        if max_total_base_bytes == 0 {
            DEFAULT_MAX_TOTAL_BASE_BYTES
        } else {
            max_total_base_bytes
        },
    )
}

fn run_scan(
    config: &ScanConfig,
    rocky_config: &RockyConfig,
    max_total_base_bytes: u64,
) -> Result<ScanExecution> {
    let started = std::time::Instant::now();
    let started_at = chrono::Utc::now();
    let root = config
        .root
        .canonicalize()
        .with_context(|| format!("failed to resolve root {}", config.root.display()))?;
    let managed_policy = load_managed_policy(config, &root, started_at)?;
    validate_local_policy_controls(config, managed_policy.as_ref())?;
    let discovery_options =
        DiscoveryOptions::from_scan(config.ignore.clone(), config.max_file_bytes);
    let plan = build_scan_plan(&root, config, &discovery_options)?;

    let loaded =
        load_project_and_metadata(config, rocky_config, &root, &plan, max_total_base_bytes)?;
    let mut readiness = loaded.readiness.clone();
    let owner_resolver = OwnerResolver::load(&root, &config.owners)?;

    let policy_digest = managed_policy
        .as_ref()
        .map(|policy| policy.digest.clone())
        .unwrap_or_else(|| "local-unmanaged".into());
    let baseline = load_scan_baseline(config, &root)?;
    if let Some(baseline) = &baseline {
        validate_finding_baseline(baseline, config.platform)?;
        validate_baseline_policy_digest(baseline, &policy_digest)?;
    }
    let branch_input = BranchScanInput {
        config,
        root: &root,
        owner_resolver: &owner_resolver,
        managed_policy: managed_policy.as_ref(),
        started_at,
        baseline: baseline.as_ref(),
        policy_digest: &policy_digest,
    };
    let mut head_scan = run_branch_diagnostics(branch_input.with_scan(BranchScanRun {
        project: &loaded.project,
        analysis_files: &loaded.analysis_files,
        metadata_diagnostics: loaded.metadata_diagnostics,
        write_baseline_path: config.write_baseline_path.as_deref(),
    }))?;
    let mut diagnostics = std::mem::take(&mut head_scan.diagnostics);
    let union_sql_documents = std::mem::take(&mut head_scan.sql_documents);
    let target_docs = std::mem::take(&mut head_scan.target_documents);
    let policy_violations = std::mem::take(&mut head_scan.policy_violations);
    let waiver_violations = std::mem::take(&mut head_scan.waiver_violations);
    let baselined_findings = head_scan.baselined_findings;
    let cost_analysis = head_scan.cost_analysis.take();
    let mut cost_summary = cost_analysis
        .as_ref()
        .map(|analysis| analysis.summary.clone());

    let head_for_delta = plan
        .pr_mode
        .then(|| finding_delta_diagnostics(&diagnostics));

    let base_branch_scan = if plan.pr_mode {
        run_base_branch_scan(BaseBranchScanInput {
            config,
            rocky_config,
            root: &root,
            plan: &plan,
            owner_resolver: &owner_resolver,
            managed_policy: managed_policy.as_ref(),
            started_at,
            baseline: baseline.as_ref(),
            policy_digest: &policy_digest,
            max_total_base_bytes,
        })?
    } else {
        None
    };

    if plan.pr_mode {
        if let (Some(summary), Some(analysis), Some(cost_config)) = (
            cost_summary.as_mut(),
            cost_analysis.as_ref(),
            config.cost.as_ref(),
        ) {
            if cost_config.enabled {
                if let Some(base_summary) = base_branch_scan
                    .as_ref()
                    .and_then(|scan| scan.cost_summary.clone())
                {
                    let downstream_ids = loaded
                        .project
                        .graph
                        .models
                        .keys()
                        .map(|id| {
                            (
                                id.clone(),
                                loaded
                                    .project
                                    .graph
                                    .transitive_downstream(std::slice::from_ref(id)),
                            )
                        })
                        .collect();
                    let changed_paths: Vec<PathBuf> = plan.changed_paths.iter().cloned().collect();
                    let blast_radius = compute_blast_radius(
                        &analysis.model_index,
                        &downstream_ids,
                        &changed_paths,
                        cost_config,
                    );
                    summary.pr_impact = Some(compute_pr_impact(
                        summary,
                        &base_summary,
                        cost_config,
                        blast_radius,
                    ));
                }
            }
        }
    }

    diagnostics.sort_by(crate::pipeline::compare_diagnostics);

    let target_counts = ScanCounts::from_files(&loaded.project.files);
    let target_parse_status = build_file_parse_status(&target_docs);
    let mut metrics = build_scan_metrics(
        &target_docs,
        &diagnostics,
        target_counts.clone(),
        loaded.metadata_only,
        loaded.metadata_warnings_len,
        loaded.yaml_parse_failures,
        loaded.dbt_project_parse_failures,
    );
    metrics.baselined_findings = baselined_findings;
    metrics.new_findings = diagnostics.len();
    readiness.analyzable_files = metrics.counts.sql + metrics.counts.yaml + metrics.counts.python;
    readiness.sql_parse_failures = metrics.sql_parse_failures;
    readiness.sql_parse_other_failures = metrics.sql_parse_other_failures;
    readiness.sql_parse_compiled_failures = metrics.sql_parse_compiled_failures;

    let context_report = if plan.pr_mode {
        Some(build_context_report(
            &plan,
            &union_sql_documents,
            &loaded.context_skip_warnings,
            &root,
        ))
    } else {
        None
    };

    let pr_summary = build_pr_summary(PrSummaryInput {
        config,
        plan: &plan,
        project: &loaded.project,
        owner_resolver: &owner_resolver,
        diagnostics: &diagnostics,
        head_for_delta: head_for_delta.as_deref(),
        base_branch_scan: base_branch_scan.as_ref(),
        cost_summary: cost_summary.as_ref(),
        checksum_mismatch_count: loaded.checksum_mismatch_count,
        rocky_integrity: &loaded.rocky_integrity,
        rocky_config,
    });

    let mut analysis = collect_analysis_violations(AnalysisViolationInput {
        config,
        metrics: &metrics,
        target_skipped_files: plan.target_skips.len(),
        metadata_only: loaded.metadata_only,
        manifest_stale: loaded.manifest_stale,
        pr_mode: plan.pr_mode,
        context: context_report.as_ref(),
        policy_violations,
        waiver_violations,
        cost_summary: cost_summary.as_ref(),
    });
    let base_rocky_required_failure = plan.pr_mode
        && rocky_config.configured
        && (config.analysis.policy == crate::AnalysisPolicy::Strict
            || rocky_config.require_artifact_integrity)
        && base_branch_scan.as_ref().is_none_or(|scan| {
            !matches!(scan.rocky_state, RockyIntegrityState::Verified)
                || !scan.rocky_messages.is_empty()
        });
    if loaded.rocky_required_failure || base_rocky_required_failure {
        analysis.violations.push(crate::AnalysisViolation {
            code: "rocky_artifact_integrity".into(),
            message: loaded
                .rocky_integrity
                .messages
                .first()
                .cloned()
                .or_else(|| {
                    base_branch_scan
                        .as_ref()
                        .and_then(|scan| scan.rocky_messages.first().cloned())
                })
                .unwrap_or_else(|| "Rocky artifact integrity is required".into()),
            observed: 1.0,
            allowed: 0.0,
        });
        analysis.passed = false;
    }
    let completed_at = chrono::Utc::now();
    Ok(ScanExecution {
        result: ScanResult {
            run: RunMetadata {
                id: format!(
                    "scan-{}-{}",
                    started_at.timestamp_millis(),
                    std::process::id()
                ),
                started_at: started_at.to_rfc3339(),
                completed_at: completed_at.to_rfc3339(),
                duration_ms: started.elapsed().as_millis().try_into().unwrap_or(u64::MAX),
                tool_version: env!("CARGO_PKG_VERSION").to_string(),
            },
            policy: managed_policy
                .as_ref()
                .map(ManagedPolicy::metadata)
                .unwrap_or_else(|| PolicyMetadata {
                    digest: "local-unmanaged".into(),
                    version: env!("CARGO_PKG_VERSION").into(),
                    scope: "local".into(),
                }),
            diagnostics,
            counts: metrics.counts.clone(),
            metrics,
            file_parse_status: target_parse_status,
            pr_summary,
            context: context_report,
            cost_summary,
            analysis,
            identity_scheme: Some(costguard_protocol::IDENTITY_SCHEME_SEMANTIC_V1.into()),
        },
        readiness,
    })
}

struct ProjectLoad {
    project: LoadedProject,
    analysis_files: Vec<costguard_scanner::ProjectFile>,
    metadata_diagnostics: Vec<costguard_diagnostics::Diagnostic>,
    context_skip_warnings: Vec<MetadataWarning>,
    metadata_only: bool,
    manifest_stale: bool,
    checksum_mismatch_count: usize,
    yaml_parse_failures: usize,
    dbt_project_parse_failures: usize,
    metadata_warnings_len: usize,
    rocky_integrity: RockyIntegrity,
    rocky_required_failure: bool,
    readiness: ReadinessFacts,
}

fn load_project_and_metadata(
    config: &ScanConfig,
    rocky_config: &RockyConfig,
    root: &Path,
    plan: &ScanPlan,
    max_total_base_bytes: u64,
) -> Result<ProjectLoad> {
    let mut union_files = plan.union_files();
    let (rocky_project, rocky_integrity, rocky_messages) =
        load_head_rocky(root, rocky_config, max_total_base_bytes)?;
    let rocky_owned = rocky_project
        .as_ref()
        .map(|project| {
            project
                .source_by_name
                .values()
                .cloned()
                .collect::<std::collections::BTreeSet<_>>()
        })
        .unwrap_or_else(|| fallback_rocky_sql_paths(rocky_config, &union_files));
    let dbt_files = union_files
        .iter()
        .filter(|file| !rocky_owned.contains(&file.root_relative_path))
        .cloned()
        .collect::<Vec<_>>();
    let dbt_load = load_dbt_project(root, config, &dbt_files)?;
    let metadata_warning_kinds = dbt_load
        .warnings
        .iter()
        .map(|warning| warning.kind.clone())
        .collect::<Vec<_>>();
    let (manifest_checksum_checked, manifest_checksum_mismatches) = dbt_load
        .project
        .as_ref()
        .map(|project| manifest_checksum_counts(project, &dbt_files))
        .unwrap_or_default();
    let dbt_project_present = crate::dbt_load::scan_roots(root, &config.paths)
        .iter()
        .any(|scan_root| scan_root.join("dbt_project.yml").is_file());
    let manifest_path = dbt_load.manifest_path.clone();
    let mut checksum_warnings = Vec::new();
    if plan.pr_mode {
        if let Some(dbt) = &dbt_load.project {
            checksum_warnings = detect_checksum_mismatches(dbt, &dbt_files, &plan.changed_paths);
        }
    }
    let metadata_only = dbt_load.metadata_only;
    let manifest_stale = dbt_load
        .warnings
        .iter()
        .chain(checksum_warnings.iter())
        .any(|warning| warning.kind == MetadataWarningKind::StaleManifest);
    let checksum_mismatch_count = checksum_warnings
        .iter()
        .filter(|warning| warning.kind == MetadataWarningKind::ManifestChecksumMismatch)
        .count();
    let yaml_parse_failures = dbt_load
        .warnings
        .iter()
        .filter(|warning| warning.kind == MetadataWarningKind::YamlParseFailed)
        .count();
    let dbt_project_parse_failures = dbt_load
        .warnings
        .iter()
        .filter(|warning| {
            matches!(
                warning.kind,
                MetadataWarningKind::DbtProjectParseFailed
                    | MetadataWarningKind::DbtProjectAmbiguousModels
            )
        })
        .count();

    let (metadata_warnings, context_skip_warnings) =
        partition_metadata_warnings(&dbt_load.warnings, plan, &skipped_files_all(plan));
    let mut metadata_warnings = metadata_warnings;
    metadata_warnings.extend(checksum_warnings);
    let mut metadata_warnings_len = metadata_warnings.len();
    let mut metadata_diagnostics = metadata_diagnostics(root, &metadata_warnings);
    for message in &rocky_messages {
        metadata_diagnostics.push(rocky_metadata_diagnostic(message, rocky_config));
    }
    metadata_warnings_len += rocky_messages.len();

    let mut graph = dbt_load
        .project
        .as_ref()
        .map(costguard_dbt::DbtProject::to_project_graph)
        .unwrap_or_default();
    let mut project_files = plan
        .targets
        .iter()
        .filter(|file| rocky_project.is_none() || !rocky_owned.contains(&file.root_relative_path))
        .cloned()
        .collect::<Vec<_>>();
    let mut compiled_unmapped_paths = std::collections::BTreeSet::new();
    let mut rocky_inputs = std::collections::BTreeSet::new();
    if let Some(rocky) = &rocky_project {
        graph
            .merge(rocky.graph.clone())
            .map_err(|message| anyhow::Error::new(crate::ProjectConfigurationError(message)))?;
        rocky_inputs = rocky.sealed_inputs.clone();
        let all_rocky = rocky_global_input_changed(plan, rocky);
        for (relative, compiled) in &rocky.compiled_by_path {
            let raw = std::fs::read_to_string(root.join(relative))
                .with_context(|| format!("failed to read Rocky source {}", relative.display()))?;
            let compiled_unmapped = relative.extension().and_then(|value| value.to_str())
                == Some("rocky")
                || raw != *compiled;
            let text = if compiled_unmapped {
                compiled.clone()
            } else {
                raw
            };
            let file = costguard_scanner::ProjectFile {
                kind: costguard_scanner::FileKind::Sql,
                path: root.join(relative),
                root_relative_path: relative.clone(),
                line_index: costguard_diagnostics::LineIndex::new(&text),
                text,
            };
            union_files.retain(|existing| existing.root_relative_path != *relative);
            union_files.push(file.clone());
            let sidecar = relative.with_extension("toml");
            let selected = scan_paths_select_rocky_source(config, root, relative);
            if (!plan.pr_mode && selected)
                || (plan.pr_mode
                    && (all_rocky
                        || plan.changed_paths.contains(relative)
                        || plan.changed_paths.contains(&sidecar)))
            {
                if compiled_unmapped {
                    compiled_unmapped_paths.insert(relative.clone());
                }
                project_files.push(file);
            }
        }
    } else if rocky_config.configured {
        for file in &mut union_files {
            if fallback_rocky_path(rocky_config, &file.root_relative_path)
                && file
                    .root_relative_path
                    .extension()
                    .and_then(|value| value.to_str())
                    == Some("sql")
            {
                file.kind = costguard_scanner::FileKind::Sql;
            }
        }
        for file in &mut project_files {
            if fallback_rocky_path(rocky_config, &file.root_relative_path)
                && file
                    .root_relative_path
                    .extension()
                    .and_then(|value| value.to_str())
                    == Some("sql")
            {
                file.kind = costguard_scanner::FileKind::Sql;
            }
        }
    }
    union_files.sort_by(|left, right| left.path.cmp(&right.path));
    project_files.sort_by(|left, right| left.path.cmp(&right.path));
    let project = LoadedProject {
        project: Project {
            root: root.to_path_buf(),
            files: project_files,
            dbt: dbt_load.project,
        },
        graph,
        compiled_unmapped_paths,
        rocky_owned_paths: rocky_owned,
        rocky_inputs,
    };
    let rocky_required_failure = rocky_config.configured
        && (config.analysis.policy == crate::AnalysisPolicy::Strict
            || rocky_config.require_artifact_integrity)
        && (!matches!(rocky_integrity.head, RockyIntegrityState::Verified)
            || !rocky_messages.is_empty());
    Ok(ProjectLoad {
        project,
        analysis_files: union_files,
        metadata_diagnostics,
        context_skip_warnings,
        metadata_only,
        manifest_stale,
        checksum_mismatch_count,
        yaml_parse_failures,
        dbt_project_parse_failures,
        metadata_warnings_len,
        rocky_integrity: rocky_integrity.clone(),
        rocky_required_failure,
        readiness: ReadinessFacts {
            dbt_project_present,
            manifest_path,
            manifest_stale,
            manifest_checksum_checked,
            manifest_checksum_mismatches,
            metadata_warning_kinds,
            rocky_configured: rocky_config.configured,
            rocky_artifact_verified: matches!(rocky_integrity.head, RockyIntegrityState::Verified),
            rocky_messages: rocky_integrity.messages.clone(),
            ..ReadinessFacts::default()
        },
    })
}

fn load_head_rocky(
    root: &Path,
    config: &RockyConfig,
    max_total_base_bytes: u64,
) -> Result<(Option<RockyProject>, RockyIntegrity, Vec<String>)> {
    let mut integrity = RockyIntegrity::default();
    if !config.configured {
        return Ok((None, integrity, Vec::new()));
    }
    let Some(path) = &config.artifact_path else {
        integrity.head = RockyIntegrityState::Missing;
        let message = "Rocky is configured but no head artifact path is available".to_string();
        integrity.messages.push(message.clone());
        return Ok((None, integrity, vec![message]));
    };
    let path = if path.is_absolute() {
        path.clone()
    } else {
        root.join(path)
    };
    if !path.is_file() {
        if config.artifact_explicit {
            anyhow::bail!("Rocky artifact path does not exist: {}", path.display());
        }
        integrity.head = RockyIntegrityState::Missing;
        let message = format!(
            "Rocky artifact {} is missing; .rocky analysis is unavailable",
            path.display()
        );
        integrity.messages.push(message.clone());
        return Ok((None, integrity, vec![message]));
    }
    let artifact = match load_rocky_artifact(&path, config.max_artifact_bytes) {
        Ok(artifact) => artifact,
        Err(error) => {
            integrity.head = RockyIntegrityState::Invalid;
            integrity.mismatch_count = 1;
            let message = format!("Rocky artifact is invalid: {error:#}");
            integrity.messages.push(message.clone());
            return Ok((None, integrity, vec![message]));
        }
    };
    match verify_rocky_artifact_current(root, &artifact, max_total_base_bytes) {
        Ok(project) => {
            integrity.head = RockyIntegrityState::Verified;
            let messages = project.unresolved_dependencies.clone();
            integrity.mismatch_count = messages.len();
            integrity.messages.extend(messages.clone());
            Ok((Some(project), integrity, messages))
        }
        Err(error) => {
            integrity.head = RockyIntegrityState::Stale;
            integrity.mismatch_count = 1;
            let message = format!("Rocky artifact is unusable: {error:#}");
            integrity.messages.push(message.clone());
            Ok((None, integrity, vec![message]))
        }
    }
}

fn fallback_rocky_sql_paths(
    config: &RockyConfig,
    files: &[costguard_scanner::ProjectFile],
) -> std::collections::BTreeSet<PathBuf> {
    if !config.configured {
        return std::collections::BTreeSet::new();
    }
    files
        .iter()
        .filter(|file| {
            fallback_rocky_path(config, &file.root_relative_path)
                && file
                    .root_relative_path
                    .extension()
                    .and_then(|value| value.to_str())
                    == Some("sql")
        })
        .map(|file| file.root_relative_path.clone())
        .collect()
}

fn fallback_rocky_path(config: &RockyConfig, path: &Path) -> bool {
    path.starts_with(&config.models_dir)
}

fn scan_paths_select_rocky_source(config: &ScanConfig, root: &Path, relative: &Path) -> bool {
    if config.paths.is_empty() {
        return true;
    }
    let source = root
        .join(relative)
        .canonicalize()
        .unwrap_or_else(|_| root.join(relative));
    config.paths.iter().any(|requested| {
        let requested = if requested.is_absolute() {
            requested.clone()
        } else {
            root.join(requested)
        };
        let requested = requested.canonicalize().unwrap_or(requested);
        if requested.is_dir() {
            source.starts_with(requested)
        } else {
            source == requested
        }
    })
}

fn rocky_global_input_changed(plan: &ScanPlan, rocky: &RockyProject) -> bool {
    let direct = rocky
        .source_by_name
        .values()
        .flat_map(|path| [path.clone(), path.with_extension("toml")])
        .collect::<std::collections::BTreeSet<_>>();
    plan.changed_paths
        .iter()
        .any(|path| rocky.sealed_inputs.contains(path) && !direct.contains(path))
}

fn rocky_metadata_diagnostic(
    message: &str,
    config: &RockyConfig,
) -> costguard_diagnostics::Diagnostic {
    let mut diagnostic = costguard_diagnostics::Diagnostic::new(
        "SQLCOST047",
        costguard_diagnostics::Severity::Low,
        config.config_path.clone(),
        None,
        message,
    )
    .with_confidence(costguard_diagnostics::Confidence::High);
    diagnostic.governance.evidence_key = costguard_diagnostics::EvidenceBuilder::new("metadata")
        .field("kind", "rocky_artifact_integrity")
        .field(
            "subject",
            costguard_diagnostics::posix_path(&config.config_path),
        )
        .field("message", message)
        .build();
    diagnostic
}

struct PrSummaryInput<'a> {
    config: &'a ScanConfig,
    plan: &'a ScanPlan,
    project: &'a LoadedProject,
    owner_resolver: &'a OwnerResolver,
    diagnostics: &'a [costguard_diagnostics::Diagnostic],
    head_for_delta: Option<&'a [costguard_diagnostics::Diagnostic]>,
    base_branch_scan: Option<&'a BaseBranchScan>,
    cost_summary: Option<&'a costguard_cost::ProjectCostSummary>,
    checksum_mismatch_count: usize,
    rocky_integrity: &'a RockyIntegrity,
    rocky_config: &'a RockyConfig,
}

fn build_pr_summary(input: PrSummaryInput<'_>) -> Option<PrSummary> {
    let PrSummaryInput {
        config,
        plan,
        project,
        owner_resolver,
        diagnostics,
        head_for_delta,
        base_branch_scan,
        cost_summary,
        checksum_mismatch_count,
        rocky_integrity,
        rocky_config,
    } = input;
    if !plan.pr_mode {
        return None;
    }
    let mut changed_files = plan.changed_paths.iter().cloned().collect::<Vec<_>>();
    changed_files.sort();
    let mut summary = enrich_pr_summary(
        PrSummary {
            changed_files,
            ..PrSummary::default()
        },
        project,
        owner_resolver,
    );
    if let Some(base_scan) = base_branch_scan {
        let rocky_comparison_complete = matches!(
            (&rocky_integrity.head, &base_scan.rocky_state),
            (RockyIntegrityState::Verified, RockyIntegrityState::Verified)
        );
        let (comparable_head, comparable_base) =
            if rocky_config.configured && !rocky_comparison_complete {
                (
                    head_for_delta
                        .unwrap_or(diagnostics)
                        .iter()
                        .filter(|diagnostic| !is_rocky_diagnostic(project, diagnostic))
                        .cloned()
                        .collect::<Vec<_>>(),
                    base_scan
                        .diagnostics
                        .iter()
                        .filter(|diagnostic| {
                            !base_scan
                                .rocky_finding_ids
                                .contains(&diagnostic.governance.finding_id)
                        })
                        .cloned()
                        .collect::<Vec<_>>(),
                )
            } else {
                (
                    head_for_delta.unwrap_or(diagnostics).to_vec(),
                    base_scan.diagnostics.clone(),
                )
            };
        summary.finding_delta = Some(classify_findings(&comparable_head, &comparable_base));
    }
    summary.gate_results =
        evaluate_gates(config.gate.as_ref(), diagnostics, &summary, cost_summary);
    summary.manifest_integrity = Some(ManifestIntegrity {
        checksum_mismatches: checksum_mismatch_count,
        verified: checksum_mismatch_count == 0,
    });
    let mut rocky_status = rocky_integrity.clone();
    if let Some(base_scan) = base_branch_scan {
        rocky_status.base = base_scan.rocky_state.clone();
        rocky_status
            .messages
            .extend(base_scan.rocky_messages.clone());
        if matches!(base_scan.rocky_state, RockyIntegrityState::Invalid) {
            rocky_status.mismatch_count += 1;
        } else if matches!(base_scan.rocky_state, RockyIntegrityState::Verified) {
            rocky_status.mismatch_count += base_scan.rocky_messages.len();
        }
        rocky_status.comparison_complete = matches!(
            (&rocky_status.head, &rocky_status.base),
            (RockyIntegrityState::Verified, RockyIntegrityState::Verified)
        );
        if !rocky_status.comparison_complete {
            rocky_status.unclassified_finding_ids = diagnostics
                .iter()
                .filter(|diagnostic| is_rocky_diagnostic(project, diagnostic))
                .map(|diagnostic| diagnostic.governance.finding_id.clone())
                .collect();
            rocky_status.unclassified_head_findings = rocky_status.unclassified_finding_ids.len();
        }
    }
    if rocky_config.configured {
        summary.rocky_artifact_integrity = Some(rocky_status);
    }
    summary.enforcement_preview = Some(EnforcementPreview {
        block_only_new: config.gate.as_ref().is_some_and(|gate| gate.block_only_new),
        require_manifest_integrity: config.analysis.require_manifest_integrity,
        require_rocky_artifact_integrity: rocky_config.require_artifact_integrity
            || config.analysis.policy == crate::AnalysisPolicy::Strict,
    });
    Some(summary)
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

fn load_scan_baseline(config: &ScanConfig, root: &Path) -> Result<Option<crate::FindingBaseline>> {
    if let Some(path) = &config.baseline_path {
        let baseline = load_finding_baseline(
            if path.is_absolute() {
                path.clone()
            } else {
                root.join(path)
            }
            .as_path(),
        )?;
        Ok(Some(baseline))
    } else {
        Ok(None)
    }
}

/// Scan a single file path (equivalent to `scan` with `paths` narrowed to one file).
pub fn explain(config: &ScanConfig, path: &Path) -> Result<ScanResult> {
    let mut config = config.clone();
    config.paths = vec![path.to_path_buf()];
    scan(&config)
}

pub fn explain_resolved(request: &ResolvedScanRequest, path: &Path) -> Result<ScanResult> {
    let mut request = request.clone();
    request.config.paths = vec![path.to_path_buf()];
    scan_resolved(&request)
}

/// Return metadata for all registered SQLCOST rules.
pub fn rules() -> Vec<costguard_rules::RuleMetadata> {
    costguard_rules::RuleRegistry::default_rules().metadata()
}
