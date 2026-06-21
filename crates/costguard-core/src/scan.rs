use crate::baseline::{load_finding_baseline, validate_finding_baseline, write_finding_baseline};
use crate::config::ScanConfig;
use crate::context::{ContextIssue, ContextReport};
use crate::dbt_graph::enrich_pr_summary;
use crate::dbt_load::{
    detect_checksum_mismatches, enrich_dbt_project_from_files, load_dbt_project,
};
use crate::gates::evaluate_gates;
use crate::governance::{
    apply_managed_governance, load_managed_policy, normalized_path, validate_local_policy_controls,
    ManagedPolicy,
};
use crate::owners::{assign_diagnostic_owners, OwnerResolver};
use crate::pipeline::{
    apply_baseline_filter, apply_enabled_and_severity, apply_inline_suppressions,
    assign_semantic_identities, effective_rule_overrides, suppression_scope,
    validate_rule_registration, IdentityResult, SuppressionScope,
};
use crate::scan_plan::{build_scan_plan, ScanPlan};
use crate::sql_analysis::{
    analyze_sql_documents, build_file_parse_status, build_scan_metrics, dbt_models_by_path,
    parse_failure_diagnostics,
};
use crate::state_diff::classify_findings;
use crate::waivers::apply_local_waivers;
use crate::{
    AnalysisReport, AnalysisViolation, EnforcementPreview, ManifestIntegrity, PolicyMetadata,
    PrSummary, Project, RunMetadata, ScanResult,
};
use anyhow::{Context, Result};
use costguard_cost::{
    build_downstream_ids, compute_blast_radius, compute_pr_impact, is_infrastructure_rule,
    run_cost_analysis, summarize_features, CostAnalysisResult, CostInputs, ModelFeatureSummary,
};
use costguard_dbt::{
    apply_dbt_project_configs_from_files, compiled_code_by_model_path,
    parse_dbt_project_with_warnings, parse_manifest_text, parse_manifest_with_limit, DbtProject,
    MetadataWarning, MetadataWarningKind,
};
use costguard_diagnostics::{EvidenceBuilder, Severity};
use costguard_protocol::EnforcementOutcome;
use costguard_rules::{ProjectIndexes, RuleRegistry};
use costguard_scanner::{DiscoveryOptions, ProjectFile, ScanCounts, SkippedFile};
use costguard_sql::{JoinKind, SqlDocument};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Run a full project scan and return diagnostics, metrics, and optional cost summary.
pub fn scan(config: &ScanConfig) -> Result<ScanResult> {
    crate::validate_scan_config(config)?;
    run_scan(config)
}

fn run_scan(config: &ScanConfig) -> Result<ScanResult> {
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

    let union_files = plan.union_files();
    let dbt_load = load_dbt_project(&root, config, &union_files)?;
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
        partition_metadata_warnings(&dbt_load.warnings, &plan, &skipped_files_all(&plan));
    let mut metadata_warnings = metadata_warnings;
    metadata_warnings.extend(checksum_warnings);
    let metadata_diagnostics = metadata_diagnostics(&root, &metadata_warnings);

    let dbt = dbt_load.project;
    let project = Project {
        root: root.clone(),
        files: plan.targets.clone(),
        dbt,
    };
    let owner_resolver = OwnerResolver::load(&root, &config.owners)?;

    let policy_digest = managed_policy
        .as_ref()
        .map(|policy| policy.digest.clone())
        .unwrap_or_else(|| "local-unmanaged".into());
    let baseline = load_scan_baseline(config, &root)?;
    if let Some(baseline) = &baseline {
        validate_finding_baseline(baseline, config.platform)?;
        validate_baseline_policy(baseline, &policy_digest)?;
    }
    let mut head_scan = run_branch_diagnostics(
        config,
        &root,
        &project,
        &union_files,
        metadata_diagnostics,
        &owner_resolver,
        managed_policy.as_ref(),
        started_at,
        baseline.as_ref(),
        config.write_baseline_path.as_deref(),
        &policy_digest,
    )?;
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
        run_base_branch_scan(
            config,
            &root,
            &plan,
            &owner_resolver,
            managed_policy.as_ref(),
            started_at,
            baseline.as_ref(),
            &policy_digest,
        )?
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
                    let downstream_ids = project
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
        metadata_only,
        metadata_warnings.len(),
        yaml_parse_failures,
        dbt_project_parse_failures,
    );
    metrics.baselined_findings = baselined_findings;
    metrics.new_findings = diagnostics.len();

    let context_report = if plan.pr_mode {
        Some(build_context_report(
            &plan,
            &union_sql_documents,
            &context_skip_warnings,
            &root,
        ))
    } else {
        None
    };

    let mut pr_summary = if plan.pr_mode {
        let mut changed_files = plan.changed_paths.iter().cloned().collect::<Vec<_>>();
        changed_files.sort();
        Some(enrich_pr_summary(
            PrSummary {
                changed_files,
                ..PrSummary::default()
            },
            &project,
            &owner_resolver,
        ))
    } else {
        None
    };
    if let Some(summary) = pr_summary.as_mut() {
        summary.gate_results = evaluate_gates(config.gate.as_ref(), &diagnostics, summary);
        if let Some(base_scan) = &base_branch_scan {
            summary.finding_delta = Some(classify_findings(
                head_for_delta.as_deref().unwrap_or(&diagnostics),
                &base_scan.diagnostics,
            ));
        }
        summary.manifest_integrity = Some(ManifestIntegrity {
            checksum_mismatches: checksum_mismatch_count,
            verified: checksum_mismatch_count == 0,
        });
        summary.enforcement_preview = Some(EnforcementPreview {
            block_only_new: config.gate.as_ref().is_some_and(|gate| gate.block_only_new),
            require_manifest_integrity: config.analysis.require_manifest_integrity,
        });
    }

    let mut analysis = evaluate_analysis(
        config,
        &metrics,
        plan.target_skips.len(),
        metadata_only,
        manifest_stale,
        plan.pr_mode,
        context_report.as_ref(),
    );
    for violation in policy_violations {
        analysis.violations.push(AnalysisViolation {
            code: violation.code,
            message: violation.message,
            observed: 1.0,
            allowed: 0.0,
        });
    }
    analysis.violations.extend(waiver_violations);
    if let Some(violation) = evaluate_cost_coverage(config.cost.as_ref(), cost_summary.as_ref()) {
        analysis.violations.push(violation);
    }
    analysis.passed = analysis.violations.is_empty();
    let completed_at = chrono::Utc::now();
    Ok(ScanResult {
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
    })
}

fn validate_baseline_policy(baseline: &crate::FindingBaseline, digest: &str) -> Result<()> {
    if let Some(expected) = &baseline.policy_digest {
        if expected != digest {
            anyhow::bail!(
                "baseline policy digest '{}' does not match active policy '{}'",
                expected,
                digest
            );
        }
    }
    Ok(())
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

fn write_baseline_from_scan(
    config: &ScanConfig,
    root: &Path,
    diagnostics: &[costguard_diagnostics::Diagnostic],
    policy_digest: &str,
    path: &Path,
) -> Result<()> {
    let output = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };
    let baseline_to_write = if let Some(existing_path) = &config.baseline_path {
        let existing = load_finding_baseline(
            if existing_path.is_absolute() {
                existing_path.clone()
            } else {
                root.join(existing_path)
            }
            .as_path(),
        )?;
        validate_finding_baseline(&existing, config.platform)?;
        validate_baseline_policy(&existing, policy_digest)?;
        crate::baseline::merge_baseline_findings(
            &existing,
            diagnostics,
            config.platform,
            Some(policy_digest.into()),
        )
    } else {
        crate::FindingBaseline::from_diagnostics(
            diagnostics,
            config.platform,
            Some(policy_digest.into()),
        )
    };
    write_finding_baseline(&output, &baseline_to_write)
}

fn evaluate_cost_coverage(
    cost_config: Option<&costguard_cost::CostConfig>,
    summary: Option<&costguard_cost::ProjectCostSummary>,
) -> Option<AnalysisViolation> {
    let cost_config = cost_config.filter(|config| config.enabled)?;
    let threshold = cost_config.min_mapped_spend_fraction?;
    let summary = summary?;
    if summary.coverage.models_total == 0 {
        return None;
    }
    let observed = summary.coverage.mapped_spend_fraction;
    if observed >= threshold {
        return None;
    }
    Some(AnalysisViolation {
        code: "cost_coverage".into(),
        message: format!(
            "mapped spend coverage {:.0}% is below required {:.0}% ({}/{} models with measured spend)",
            observed * 100.0,
            threshold * 100.0,
            summary.coverage.models_with_mapped_spend,
            summary.coverage.models_total
        ),
        observed,
        allowed: threshold,
    })
}

struct BranchDiagnostics {
    diagnostics: Vec<costguard_diagnostics::Diagnostic>,
    sql_documents: Vec<SqlDocument>,
    target_documents: Vec<SqlDocument>,
    policy_violations: Vec<costguard_policy::PolicyViolation>,
    waiver_violations: Vec<AnalysisViolation>,
    baselined_findings: usize,
    cost_analysis: Option<CostAnalysisResult>,
}

#[allow(clippy::too_many_arguments)]
fn run_branch_diagnostics(
    config: &ScanConfig,
    root: &Path,
    project: &Project,
    analysis_files: &[ProjectFile],
    metadata_diagnostics: Vec<costguard_diagnostics::Diagnostic>,
    owner_resolver: &OwnerResolver,
    managed_policy: Option<&ManagedPolicy>,
    started_at: chrono::DateTime<chrono::Utc>,
    baseline: Option<&crate::FindingBaseline>,
    write_baseline_path: Option<&Path>,
    policy_digest: &str,
) -> Result<BranchDiagnostics> {
    let compiled_by_path = project
        .dbt
        .as_ref()
        .map(compiled_code_by_model_path)
        .unwrap_or_default();
    let sql_documents = analyze_sql_documents(analysis_files, config.platform, &compiled_by_path);
    let project_indexes = ProjectIndexes::from_sql_documents(&sql_documents);
    let sql_by_path = sql_documents
        .iter()
        .map(|doc| (doc.path.clone(), doc))
        .collect::<HashMap<_, _>>();
    let dbt_by_path = dbt_models_by_path(project);
    let registry = RuleRegistry::default_rules();
    validate_rule_registration(&registry)?;

    let mut raw_diagnostics = metadata_diagnostics;
    let file_texts = file_text_map(&project.files);
    let file_scopes = file_scope_map(&project.files);
    for file in &project.files {
        let sql = sql_by_path.get(&file.path).copied();
        let dbt_model = dbt_by_path.get(&file.root_relative_path).copied();
        let policy_path = normalized_path(&file.root_relative_path);
        let resolved = managed_policy
            .map(|policy| policy.resolve(Some(&policy_path)))
            .transpose()?;
        let overrides = effective_rule_overrides(config, resolved.as_ref());
        let ctx = costguard_rules::RuleContext {
            warehouse: config.platform,
            file,
            sql,
            dbt_model,
            all_sql: &sql_documents,
            project_indexes: &project_indexes,
            overrides: &overrides,
        };
        let mut file_diagnostics = registry.run(&ctx);
        if let Some(policy) = &resolved {
            file_diagnostics.extend(registry.run_declarative(&ctx, &policy.custom_rules)?);
        }
        raw_diagnostics.extend(apply_enabled_and_severity(file_diagnostics, &overrides));
    }

    let target_paths = project
        .files
        .iter()
        .map(|file| &file.path)
        .collect::<std::collections::HashSet<_>>();
    let target_documents = sql_documents
        .iter()
        .filter(|document| target_paths.contains(&document.path))
        .cloned()
        .collect::<Vec<_>>();
    raw_diagnostics.extend(parse_failure_diagnostics(&target_documents, root));

    let allow_inline =
        managed_policy.is_none_or(|policy| policy.document.permissions.allow_inline_suppressions);
    let diagnostics =
        apply_inline_suppressions(raw_diagnostics, &file_texts, &file_scopes, allow_inline);
    let IdentityResult { mut diagnostics } = assign_semantic_identities(diagnostics)?;
    let policy_violations = apply_managed_governance(&mut diagnostics, managed_policy, started_at)?;
    assign_diagnostic_owners(&mut diagnostics, owner_resolver, project.dbt.as_ref());
    let waiver_violations = apply_local_waivers(&mut diagnostics, &config.waivers, started_at);

    if let Some(path) = write_baseline_path {
        write_baseline_from_scan(config, root, &diagnostics, policy_digest, path)?;
    }
    let (diagnostics, baselined_findings) = apply_baseline_filter(diagnostics, baseline);
    let mut diagnostics = crate::pipeline::filter_by_min_confidence(
        diagnostics,
        config.min_confidence,
        config.min_confidence_filter,
    );
    for diagnostic in &mut diagnostics {
        if let Some(tier) = costguard_protocol::precision_tier(&diagnostic.rule_id) {
            diagnostic.rule_precision_tier = Some(tier.to_string());
        }
    }
    let features_by_path = feature_summaries_by_path(&sql_documents);
    let cost_analysis =
        run_optional_cost(config, root, project, &mut diagnostics, &features_by_path)?;
    Ok(BranchDiagnostics {
        diagnostics,
        sql_documents,
        target_documents,
        policy_violations,
        waiver_violations,
        baselined_findings,
        cost_analysis,
    })
}

fn finding_delta_diagnostics(
    diagnostics: &[costguard_diagnostics::Diagnostic],
) -> Vec<costguard_diagnostics::Diagnostic> {
    diagnostics
        .iter()
        .filter(|diagnostic| {
            diagnostic.governance.enforcement != EnforcementOutcome::Excepted
                && !is_infrastructure_rule(&diagnostic.rule_id)
        })
        .cloned()
        .collect()
}

fn run_optional_cost(
    config: &ScanConfig,
    root: &Path,
    project: &Project,
    diagnostics: &mut [costguard_diagnostics::Diagnostic],
    features_by_path: &HashMap<PathBuf, ModelFeatureSummary>,
) -> Result<Option<CostAnalysisResult>> {
    if let Some(cost_config) = &config.cost {
        if cost_config.enabled {
            let inputs = CostInputs::load(root, cost_config)?;
            return Ok(Some(run_cost_analysis(
                cost_config,
                project.dbt.as_ref(),
                &inputs,
                diagnostics,
                features_by_path,
            )));
        }
    }
    Ok(None)
}

struct BaseBranchScan {
    diagnostics: Vec<costguard_diagnostics::Diagnostic>,
    cost_summary: Option<costguard_cost::ProjectCostSummary>,
}

#[allow(clippy::too_many_arguments)]
fn run_base_branch_scan(
    config: &ScanConfig,
    root: &Path,
    plan: &ScanPlan,
    owner_resolver: &OwnerResolver,
    managed_policy: Option<&ManagedPolicy>,
    started_at: chrono::DateTime<chrono::Utc>,
    baseline: Option<&crate::FindingBaseline>,
    policy_digest: &str,
) -> Result<Option<BaseBranchScan>> {
    if !plan.pr_mode {
        return Ok(None);
    }
    let commit = plan
        .base_commit
        .as_deref()
        .context("PR scan plan is missing its resolved base commit")?;
    let base_dbt = load_base_dbt_project(root, commit, config)?;
    let base_files = load_base_branch_files(root, commit, plan, config)?;
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
        config,
        root,
        &project,
        &base_files.context,
        Vec::new(),
        owner_resolver,
        managed_policy,
        started_at,
        baseline,
        None,
        policy_digest,
    )?;
    Ok(Some(BaseBranchScan {
        diagnostics: finding_delta_diagnostics(&branch.diagnostics),
        cost_summary: branch.cost_analysis.map(|analysis| analysis.summary),
    }))
}

fn load_base_dbt_project(
    root: &Path,
    commit: &str,
    config: &ScanConfig,
) -> Result<Option<DbtProject>> {
    if let Some(path) = &config.base_manifest_path {
        let resolved = if path.is_absolute() {
            path.clone()
        } else {
            root.join(path)
        };
        return Ok(Some(parse_manifest_with_limit(
            &resolved,
            config.max_manifest_bytes,
        )?));
    }
    let manifest_rel = base_manifest_rel_path(root, config);
    let mut files = crate::git::files_at_commit_with_limit(
        root,
        commit,
        &[(manifest_rel.clone(), config.max_manifest_bytes)],
    )?;
    let Some(text) = files.remove(&manifest_rel).flatten() else {
        return Ok(None);
    };
    Ok(Some(parse_manifest_text(&text)?))
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

fn load_base_branch_files(
    root: &Path,
    commit: &str,
    plan: &ScanPlan,
    config: &ScanConfig,
) -> Result<BaseBranchFiles> {
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
    let max_file_bytes = costguard_scanner::effective_max_file_bytes(config.max_file_bytes);
    let requests = rel_paths
        .iter()
        .cloned()
        .map(|path| (path, max_file_bytes))
        .collect::<Vec<_>>();
    let mut contents = crate::git::files_at_commit_with_limit(root, commit, &requests)?;
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
    Ok(BaseBranchFiles { targets, context })
}

fn base_path_is_ignored(root: &Path, path: &Path, ignored: &[PathBuf]) -> bool {
    ignored.iter().any(|ignored| {
        let relative = ignored.strip_prefix(root).unwrap_or(ignored);
        path == relative || path.starts_with(relative)
    })
}

fn evaluate_analysis(
    config: &ScanConfig,
    metrics: &crate::ScanMetrics,
    target_skipped_files: usize,
    metadata_only: bool,
    manifest_stale: bool,
    pr_mode: bool,
    context: Option<&ContextReport>,
) -> AnalysisReport {
    let analysis = config.analysis.effective();
    let mut violations = Vec::new();
    let parse_total = metrics.sql_parse_total + metrics.sql_parse_other_total;
    let parse_failures = metrics.sql_parse_failures + metrics.sql_parse_other_failures;
    let parse_failure_rate = if parse_total == 0 {
        0.0
    } else {
        parse_failures as f64 / parse_total as f64
    };
    if parse_failure_rate > analysis.max_parse_failure_rate {
        violations.push(AnalysisViolation {
            code: "parse_failure_rate".into(),
            message: format!(
                "SQL parse failure rate {:.2}% exceeds allowed {:.2}%",
                parse_failure_rate * 100.0,
                analysis.max_parse_failure_rate * 100.0
            ),
            observed: parse_failure_rate,
            allowed: analysis.max_parse_failure_rate,
        });
    }
    if metrics.sql_parse_compiled_failures > analysis.max_compiled_parse_failures {
        violations.push(AnalysisViolation {
            code: "compiled_parse_failures".into(),
            message: format!(
                "{} compiled SQL parse failures exceed allowed {}",
                metrics.sql_parse_compiled_failures, analysis.max_compiled_parse_failures
            ),
            observed: metrics.sql_parse_compiled_failures as f64,
            allowed: analysis.max_compiled_parse_failures as f64,
        });
    }
    if target_skipped_files > analysis.max_skipped_files {
        violations.push(AnalysisViolation {
            code: "skipped_files".into(),
            message: format!(
                "{target_skipped_files} skipped or unreadable target files exceed allowed {}",
                analysis.max_skipped_files
            ),
            observed: target_skipped_files as f64,
            allowed: analysis.max_skipped_files as f64,
        });
    }
    let metadata_errors = metrics.yaml_parse_failures + metrics.dbt_project_parse_failures;
    if analysis.fail_on_metadata_errors && metadata_errors > 0 {
        violations.push(AnalysisViolation {
            code: "metadata_errors".into(),
            message: format!("{metadata_errors} metadata parse errors make analysis incomplete"),
            observed: metadata_errors as f64,
            allowed: 0.0,
        });
    }
    if analysis.require_manifest && metadata_only {
        violations.push(AnalysisViolation {
            code: "manifest_required".into(),
            message: "a dbt manifest is required for this analysis policy; run 'dbt compile' to produce target/manifest.json or pass --manifest <path>".into(),
            observed: 0.0,
            allowed: 1.0,
        });
    }
    if analysis.require_manifest && manifest_stale {
        violations.push(AnalysisViolation {
            code: "manifest_stale".into(),
            message: "dbt manifest is stale (older than modified models); run 'dbt compile' before scanning".into(),
            observed: 1.0,
            allowed: 0.0,
        });
    }
    if pr_mode {
        let _ = context;
    }
    AnalysisReport {
        policy: analysis.policy,
        passed: violations.is_empty(),
        violations,
    }
}

fn feature_summaries_by_path(documents: &[SqlDocument]) -> HashMap<PathBuf, ModelFeatureSummary> {
    documents
        .iter()
        .map(|doc| {
            let cte_reuse_max = doc
                .features
                .cte_references
                .iter()
                .map(|cte| cte.key.parse::<usize>().unwrap_or(1))
                .max()
                .unwrap_or(0);
            (
                doc.path.clone(),
                summarize_features(
                    doc.features.joins.len(),
                    doc.features
                        .joins
                        .iter()
                        .filter(|join| matches!(join.kind, JoinKind::Cross | JoinKind::Comma))
                        .count(),
                    doc.features.select_stars.len(),
                    doc.features.window_functions.len(),
                    cte_reuse_max,
                ),
            )
        })
        .collect()
}

/// Scan a single file path (equivalent to `scan` with `paths` narrowed to one file).
pub fn explain(config: &ScanConfig, path: &Path) -> Result<ScanResult> {
    let mut config = config.clone();
    config.paths = vec![path.to_path_buf()];
    scan(&config)
}

/// Return metadata for all registered SQLCOST rules.
pub fn rules() -> Vec<costguard_rules::RuleMetadata> {
    RuleRegistry::default_rules().metadata()
}

fn skipped_files_all(plan: &ScanPlan) -> Vec<SkippedFile> {
    let mut all = plan.target_skips.clone();
    all.extend(plan.context_skips.clone());
    all
}

fn partition_metadata_warnings(
    warnings: &[MetadataWarning],
    plan: &ScanPlan,
    skipped_files: &[SkippedFile],
) -> (Vec<MetadataWarning>, Vec<MetadataWarning>) {
    let mut target = Vec::new();
    let mut context = Vec::new();
    for warning in warnings {
        if warning.kind == MetadataWarningKind::FileSkipped {
            if let Some(path) = &warning.path {
                if plan.is_target(path) {
                    target.push(warning.clone());
                } else if plan.pr_mode {
                    context.push(warning.clone());
                }
            } else {
                target.push(warning.clone());
            }
        } else {
            target.push(warning.clone());
        }
    }
    for file in skipped_files {
        let warning = MetadataWarning {
            kind: MetadataWarningKind::FileSkipped,
            path: Some(file.path.clone()),
            message: format!(
                "skipped {}: {}",
                file.root_relative_path.display(),
                file.reason
            ),
        };
        if plan.is_target(&file.path) {
            target.push(warning);
        } else if plan.pr_mode {
            context.push(warning);
        }
    }
    (target, context)
}

fn metadata_diagnostics(
    root: &Path,
    warnings: &[MetadataWarning],
) -> Vec<costguard_diagnostics::Diagnostic> {
    warnings
        .iter()
        .map(|warning| metadata_warning_to_diagnostic(root, warning))
        .collect()
}

fn metadata_warning_to_diagnostic(
    root: &Path,
    warning: &MetadataWarning,
) -> costguard_diagnostics::Diagnostic {
    let (rule_id, severity, suggestion, kind) = match warning.kind {
        MetadataWarningKind::NoManifest => (
            "SQLCOST023",
            Severity::Info,
            "run 'dbt compile' to produce target/manifest.json, then scan again (or pass --manifest <path>)",
            "no_manifest",
        ),
        MetadataWarningKind::StaleManifest => (
            "SQLCOST045",
            Severity::Info,
            "re-run 'dbt compile' so target/manifest.json reflects current models, then scan again",
            "stale_manifest",
        ),
        MetadataWarningKind::ManifestChecksumMismatch => (
            "SQLCOST046",
            Severity::Low,
            "re-run 'dbt compile' so the manifest checksum matches current model SQL",
            "manifest_checksum_mismatch",
        ),
        MetadataWarningKind::YamlParseFailed => (
            "SQLCOST024",
            Severity::Low,
            "fix the schema YAML syntax so Costguard can read model config",
            "yaml_parse_failed",
        ),
        MetadataWarningKind::DbtProjectParseFailed
        | MetadataWarningKind::DbtProjectAmbiguousModels => (
            "SQLCOST025",
            Severity::Low,
            "fix dbt_project.yml syntax or project name alignment for folder config",
            "dbt_project_metadata",
        ),
        MetadataWarningKind::FileSkipped => (
            "SQLCOST026",
            Severity::Low,
            "narrow scan paths, ignore generated files, or raise [scan].max_file_bytes in costguard.toml",
            "file_skipped",
        ),
    };
    let path = warning
        .path
        .as_ref()
        .map(|path| {
            path.strip_prefix(root)
                .map(Path::to_path_buf)
                .unwrap_or_else(|_| path.clone())
        })
        .unwrap_or_else(|| PathBuf::from("."));
    let subject = costguard_diagnostics::posix_path(&path);
    let evidence_key = EvidenceBuilder::new("metadata")
        .field("kind", kind)
        .field("subject", subject)
        .build();
    let mut diagnostic = costguard_diagnostics::Diagnostic::new(
        rule_id,
        severity,
        path,
        None,
        warning.message.clone(),
    )
    .with_suggestion(suggestion);
    diagnostic.governance.evidence_key = evidence_key;
    diagnostic
}

fn build_context_report(
    plan: &ScanPlan,
    union_sql_documents: &[SqlDocument],
    context_skip_warnings: &[MetadataWarning],
    root: &Path,
) -> ContextReport {
    let context_docs: Vec<SqlDocument> = union_sql_documents
        .iter()
        .filter(|doc| !plan.is_target(&doc.path))
        .cloned()
        .collect();
    let mut issues = Vec::new();
    for doc in &context_docs {
        if doc.is_dbt_sql_model && !doc.parsed {
            let path = doc
                .path
                .strip_prefix(root)
                .map(Path::to_path_buf)
                .unwrap_or_else(|_| doc.path.clone());
            issues.push(ContextIssue {
                rule_id: "SQLCOST027".into(),
                path: costguard_diagnostics::posix_path(&path),
                message: format!(
                    "SQL parse failed for {}",
                    doc.path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or("model")
                ),
            });
        }
    }
    for warning in context_skip_warnings {
        let path = warning
            .path
            .as_ref()
            .map(|p| costguard_diagnostics::posix_path(p.strip_prefix(root).unwrap_or(p)))
            .unwrap_or_else(|| ".".into());
        issues.push(ContextIssue {
            rule_id: "SQLCOST026".into(),
            path,
            message: warning.message.clone(),
        });
    }
    let metrics = build_scan_metrics(
        &context_docs,
        &[],
        ScanCounts::from_files(&plan.context),
        false,
        0,
        0,
        0,
    );
    ContextReport {
        counts: ScanCounts::from_files(&plan.context),
        sql_parse_total: metrics.sql_parse_total,
        sql_parse_failures: metrics.sql_parse_failures,
        sql_parse_other_total: metrics.sql_parse_other_total,
        sql_parse_other_failures: metrics.sql_parse_other_failures,
        skipped_count: plan.context_skips.len(),
        issues,
    }
}

fn file_text_map(files: &[ProjectFile]) -> HashMap<PathBuf, String> {
    files
        .iter()
        .map(|file| (file.root_relative_path.clone(), file.text.clone()))
        .collect()
}

fn file_scope_map(files: &[ProjectFile]) -> HashMap<PathBuf, SuppressionScope> {
    files
        .iter()
        .map(|file| (file.root_relative_path.clone(), suppression_scope(file)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn infrastructure_findings_do_not_enter_finding_deltas() {
        let metadata = costguard_diagnostics::Diagnostic::new(
            "SQLCOST046",
            Severity::Low,
            PathBuf::from("target/manifest.json"),
            None,
            "stale manifest",
        );
        let behavioral = costguard_diagnostics::Diagnostic::new(
            "SQLCOST001",
            Severity::High,
            PathBuf::from("models/a.sql"),
            None,
            "select star",
        );
        let filtered = finding_delta_diagnostics(&[metadata, behavioral]);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].rule_id, "SQLCOST001");
    }
}
