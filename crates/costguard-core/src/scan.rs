use crate::analysis_report::{collect_analysis_violations, AnalysisViolationInput};
use crate::base_replay::{run_base_branch_scan, BaseBranchScan, BaseBranchScanInput};
use crate::baseline::{
    load_finding_baseline, validate_baseline_policy_digest, validate_finding_baseline,
};
use crate::branch_scan::{
    finding_delta_diagnostics, run_branch_diagnostics, BranchScanInput, BranchScanRun,
};
use crate::config::{ResolvedScanRequest, ScanConfig, DEFAULT_MAX_TOTAL_BASE_BYTES};
use crate::dbt_graph::enrich_pr_summary;
use crate::dbt_load::{detect_checksum_mismatches, load_dbt_project, manifest_checksum_counts};
use crate::gates::evaluate_gates;
use crate::governance::{load_managed_policy, validate_local_policy_controls, ManagedPolicy};
use crate::metadata_report::{
    build_context_report, metadata_diagnostics, partition_metadata_warnings, skipped_files_all,
};
use crate::owners::OwnerResolver;
use crate::scan_plan::{build_scan_plan, ScanPlan};
use crate::sql_analysis::{build_file_parse_status, build_scan_metrics};
use crate::state_diff::classify_findings;
use crate::{
    EnforcementPreview, ManifestIntegrity, PolicyMetadata, PrSummary, Project, RunMetadata,
    ScanResult,
};
use anyhow::{Context, Result};
use costguard_cost::{build_downstream_ids, compute_blast_radius, compute_pr_impact};
use costguard_dbt::{MetadataWarning, MetadataWarningKind};
use costguard_scanner::{DiscoveryOptions, ScanCounts};
use std::path::{Path, PathBuf};

/// Run a full project scan and return diagnostics, metrics, and optional cost summary.
pub fn scan(config: &ScanConfig) -> Result<ScanResult> {
    crate::validate_scan_config(config)?;
    Ok(run_scan(config, DEFAULT_MAX_TOTAL_BASE_BYTES)?.result)
}

/// Run a scan with project-resolved execution safety limits.
pub fn scan_resolved(request: &ResolvedScanRequest) -> Result<ScanResult> {
    crate::validate_scan_config(&request.config)?;
    let max_total_base_bytes = if request.max_total_base_bytes == 0 {
        DEFAULT_MAX_TOTAL_BASE_BYTES
    } else {
        request.max_total_base_bytes
    };
    Ok(run_scan(&request.config, max_total_base_bytes)?.result)
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
        if max_total_base_bytes == 0 {
            DEFAULT_MAX_TOTAL_BASE_BYTES
        } else {
            max_total_base_bytes
        },
    )
}

fn run_scan(config: &ScanConfig, max_total_base_bytes: u64) -> Result<ScanExecution> {
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

    let loaded = load_project_and_metadata(config, &root, &plan)?;
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
        analysis_files: &plan.union_files(),
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
                        .dbt
                        .as_ref()
                        .map(build_downstream_ids)
                        .unwrap_or_default();
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

    let target_counts = ScanCounts::from_files(&plan.targets);
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
    });

    let analysis = collect_analysis_violations(AnalysisViolationInput {
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
    project: Project,
    metadata_diagnostics: Vec<costguard_diagnostics::Diagnostic>,
    context_skip_warnings: Vec<MetadataWarning>,
    metadata_only: bool,
    manifest_stale: bool,
    checksum_mismatch_count: usize,
    yaml_parse_failures: usize,
    dbt_project_parse_failures: usize,
    metadata_warnings_len: usize,
    readiness: ReadinessFacts,
}

fn load_project_and_metadata(
    config: &ScanConfig,
    root: &Path,
    plan: &ScanPlan,
) -> Result<ProjectLoad> {
    let union_files = plan.union_files();
    let dbt_load = load_dbt_project(root, config, &union_files)?;
    let metadata_warning_kinds = dbt_load
        .warnings
        .iter()
        .map(|warning| warning.kind.clone())
        .collect::<Vec<_>>();
    let (manifest_checksum_checked, manifest_checksum_mismatches) = dbt_load
        .project
        .as_ref()
        .map(|project| manifest_checksum_counts(project, &union_files))
        .unwrap_or_default();
    let dbt_project_present = crate::dbt_load::scan_roots(root, &config.paths)
        .iter()
        .any(|scan_root| scan_root.join("dbt_project.yml").is_file());
    let manifest_path = dbt_load.manifest_path.clone();
    let mut checksum_warnings = Vec::new();
    if plan.pr_mode {
        if let Some(dbt) = &dbt_load.project {
            checksum_warnings = detect_checksum_mismatches(dbt, &union_files, &plan.changed_paths);
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
    let metadata_warnings_len = metadata_warnings.len();
    let metadata_diagnostics = metadata_diagnostics(root, &metadata_warnings);

    let project = Project {
        root: root.to_path_buf(),
        files: plan.targets.clone(),
        dbt: dbt_load.project,
    };
    Ok(ProjectLoad {
        project,
        metadata_diagnostics,
        context_skip_warnings,
        metadata_only,
        manifest_stale,
        checksum_mismatch_count,
        yaml_parse_failures,
        dbt_project_parse_failures,
        metadata_warnings_len,
        readiness: ReadinessFacts {
            dbt_project_present,
            manifest_path,
            manifest_stale,
            manifest_checksum_checked,
            manifest_checksum_mismatches,
            metadata_warning_kinds,
            ..ReadinessFacts::default()
        },
    })
}

struct PrSummaryInput<'a> {
    config: &'a ScanConfig,
    plan: &'a ScanPlan,
    project: &'a Project,
    owner_resolver: &'a OwnerResolver,
    diagnostics: &'a [costguard_diagnostics::Diagnostic],
    head_for_delta: Option<&'a [costguard_diagnostics::Diagnostic]>,
    base_branch_scan: Option<&'a BaseBranchScan>,
    cost_summary: Option<&'a costguard_cost::ProjectCostSummary>,
    checksum_mismatch_count: usize,
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
        summary.finding_delta = Some(classify_findings(
            head_for_delta.unwrap_or(diagnostics),
            &base_scan.diagnostics,
        ));
    }
    summary.gate_results =
        evaluate_gates(config.gate.as_ref(), diagnostics, &summary, cost_summary);
    summary.manifest_integrity = Some(ManifestIntegrity {
        checksum_mismatches: checksum_mismatch_count,
        verified: checksum_mismatch_count == 0,
    });
    summary.enforcement_preview = Some(EnforcementPreview {
        block_only_new: config.gate.as_ref().is_some_and(|gate| gate.block_only_new),
        require_manifest_integrity: config.analysis.require_manifest_integrity,
    });
    Some(summary)
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

/// Return metadata for all registered SQLCOST rules.
pub fn rules() -> Vec<costguard_rules::RuleMetadata> {
    costguard_rules::RuleRegistry::default_rules().metadata()
}
