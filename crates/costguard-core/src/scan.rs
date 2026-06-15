use crate::baseline::{load_finding_baseline, validate_finding_baseline, write_finding_baseline};
use crate::config::ScanConfig;
use crate::context::{ContextIssue, ContextReport};
use crate::dbt_graph::enrich_pr_summary;
use crate::dbt_load::load_dbt_project;
use crate::governance::{
    apply_managed_governance, load_managed_policy, normalized_path, validate_local_policy_controls,
    ManagedPolicy,
};
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
use crate::{
    AnalysisReport, AnalysisViolation, PolicyMetadata, PrSummary, Project, RunMetadata, ScanResult,
};
use anyhow::{Context, Result};
use costguard_cost::{
    compute_pr_impact, run_cost_analysis, summarize_features, CostInputs, ModelFeatureSummary,
};
use costguard_dbt::{
    compiled_code_by_model_path, extract_sql_features, MetadataWarning, MetadataWarningKind,
};
use costguard_diagnostics::{EvidenceBuilder, Severity};
use costguard_rules::{ProjectIndexes, RuleRegistry};
use costguard_scanner::{DiscoveryOptions, ProjectFile, ScanCounts, SkippedFile};
use costguard_sql::{JoinKind, SqlDocument};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Run a full project scan and return diagnostics, metrics, and optional cost summary.
pub fn scan(config: &ScanConfig) -> Result<ScanResult> {
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
    let metadata_only = dbt_load.metadata_only;
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
    let metadata_diagnostics = metadata_diagnostics(&root, &metadata_warnings);

    let dbt = dbt_load.project;
    let project = Project {
        root: root.clone(),
        files: plan.targets.clone(),
        dbt,
    };

    let compiled_by_path = project
        .dbt
        .as_ref()
        .map(compiled_code_by_model_path)
        .unwrap_or_default();
    let union_sql_documents =
        analyze_sql_documents(&union_files, config.platform, &compiled_by_path);
    let project_indexes = ProjectIndexes::from_sql_documents(&union_sql_documents);
    let sql_by_path = union_sql_documents
        .iter()
        .map(|doc| (doc.path.clone(), doc))
        .collect::<HashMap<_, _>>();
    let dbt_by_path = dbt_models_by_path(&project);
    let registry = RuleRegistry::default_rules();
    validate_rule_registration(&registry)?;

    let mut raw_diagnostics = metadata_diagnostics;
    let file_texts = file_text_map(&plan.targets);
    let file_scopes = file_scope_map(&plan.targets);

    for file in &plan.targets {
        let sql = sql_by_path.get(&file.path).copied();
        let dbt_model = dbt_by_path.get(&file.root_relative_path).copied();
        let policy_path = normalized_path(&file.root_relative_path);
        let resolved = managed_policy
            .as_ref()
            .map(|policy| policy.resolve(Some(&policy_path)))
            .transpose()?;
        let overrides = effective_rule_overrides(config, resolved.as_ref());
        let ctx = costguard_rules::RuleContext {
            warehouse: config.platform,
            file,
            sql,
            dbt_model,
            all_sql: &union_sql_documents,
            project_indexes: &project_indexes,
            overrides: &overrides,
        };
        let mut file_diagnostics = registry.run(&ctx);
        if let Some(policy) = &resolved {
            file_diagnostics.extend(registry.run_declarative(&ctx, &policy.custom_rules)?);
        }
        file_diagnostics = apply_enabled_and_severity(file_diagnostics, &overrides);
        raw_diagnostics.extend(file_diagnostics);
    }

    let target_docs: Vec<SqlDocument> = union_sql_documents
        .iter()
        .filter(|doc| plan.is_target(&doc.path))
        .cloned()
        .collect();
    raw_diagnostics.extend(parse_failure_diagnostics(&target_docs, &root));

    let allow_inline = managed_policy
        .as_ref()
        .is_none_or(|policy| policy.document.permissions.allow_inline_suppressions);
    let diagnostics =
        apply_inline_suppressions(raw_diagnostics, &file_texts, &file_scopes, allow_inline);

    let IdentityResult { mut diagnostics } = assign_semantic_identities(diagnostics)?;

    let policy_violations =
        apply_managed_governance(&mut diagnostics, managed_policy.as_ref(), started_at)?;
    let policy_digest = managed_policy
        .as_ref()
        .map(|policy| policy.digest.clone())
        .unwrap_or_else(|| "local-unmanaged".into());

    if let Some(path) = &config.write_baseline_path {
        write_baseline_from_scan(config, &root, &diagnostics, &policy_digest, path)?;
    }

    let baseline = load_scan_baseline(config, &root)?;
    if let Some(baseline) = &baseline {
        validate_finding_baseline(baseline, config.platform)?;
        validate_baseline_policy(baseline, &policy_digest)?;
    }

    let (mut diagnostics, baselined_findings) =
        apply_baseline_filter(diagnostics, baseline.as_ref());

    let features_by_path = feature_summaries_by_path(&union_sql_documents);
    let mut cost_summary =
        run_optional_cost(config, &root, &project, &mut diagnostics, &features_by_path)?;

    if plan.pr_mode {
        if let (Some(summary), Some(cost_config)) = (cost_summary.as_mut(), config.cost.as_ref()) {
            if cost_config.enabled {
                if let Some(base_summary) = run_base_branch_cost(
                    config,
                    &root,
                    &plan,
                    &project,
                    &union_sql_documents,
                    &project_indexes,
                )? {
                    summary.pr_impact =
                        Some(compute_pr_impact(summary, &base_summary, cost_config));
                }
            }
        }
    }

    diagnostics.sort_by(crate::pipeline::compare_diagnostics);

    let target_counts = ScanCounts::from_files(&plan.targets);
    let _context_counts = if plan.pr_mode {
        ScanCounts::from_files(&plan.context)
    } else {
        target_counts.clone()
    };
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

    let pr_summary = if plan.pr_mode {
        Some(enrich_pr_summary(
            PrSummary {
                changed_files: plan.changed_paths.iter().cloned().collect(),
                ..PrSummary::default()
            },
            &project,
        ))
    } else {
        None
    };

    let mut analysis = evaluate_analysis(
        config,
        &metrics,
        plan.target_skips.len(),
        metadata_only,
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

fn run_optional_cost(
    config: &ScanConfig,
    root: &Path,
    project: &Project,
    diagnostics: &mut [costguard_diagnostics::Diagnostic],
    features_by_path: &HashMap<PathBuf, ModelFeatureSummary>,
) -> Result<Option<costguard_cost::ProjectCostSummary>> {
    if let Some(cost_config) = &config.cost {
        if cost_config.enabled {
            let inputs = CostInputs::load(root, cost_config)?;
            return Ok(Some(
                run_cost_analysis(
                    cost_config,
                    project.dbt.as_ref(),
                    &inputs,
                    diagnostics,
                    features_by_path,
                )
                .summary,
            ));
        }
    }
    Ok(None)
}

fn run_base_branch_cost(
    config: &ScanConfig,
    root: &Path,
    plan: &ScanPlan,
    project: &Project,
    union_sql_documents: &[SqlDocument],
    project_indexes: &ProjectIndexes,
) -> Result<Option<costguard_cost::ProjectCostSummary>> {
    let cost_config = match &config.cost {
        Some(cost) if cost.enabled => cost,
        _ => return Ok(None),
    };
    let base = config.base_branch.as_deref().unwrap_or("main");
    let inputs = CostInputs::load(root, cost_config)?;
    let registry = RuleRegistry::default_rules();
    let dbt_by_path = dbt_models_by_path(project);
    let compiled_by_path = project
        .dbt
        .as_ref()
        .map(compiled_code_by_model_path)
        .unwrap_or_default();

    let mut base_docs = Vec::new();
    let mut base_files = Vec::new();
    for file in &plan.targets {
        if !matches!(
            file.kind,
            costguard_scanner::FileKind::Sql | costguard_scanner::FileKind::DbtSqlModel
        ) {
            continue;
        }
        let Some(text) = costguard_git::file_at_ref(root, base, &file.root_relative_path)? else {
            continue;
        };
        let line_index = costguard_diagnostics::LineIndex::new(&text);
        let doc = costguard_sql::analyze_sql(
            file.path.clone(),
            &text,
            config.platform,
            &line_index,
            compiled_by_path.get(&file.path).map(String::as_str),
            file.kind == costguard_scanner::FileKind::DbtSqlModel,
            extract_sql_features(&text),
        );
        base_docs.push(doc);
        base_files.push(file.clone());
    }

    if base_docs.is_empty() {
        return Ok(Some(costguard_cost::ProjectCostSummary::default()));
    }

    let mut base_diagnostics = Vec::new();
    for file in &base_files {
        let sql = base_docs.iter().find(|doc| doc.path == file.path);
        let dbt_model = dbt_by_path.get(&file.root_relative_path).copied();
        let ctx = costguard_rules::RuleContext {
            warehouse: config.platform,
            file,
            sql,
            dbt_model,
            all_sql: union_sql_documents,
            project_indexes,
            overrides: &costguard_rules::RuleOverrides::default(),
        };
        base_diagnostics.extend(registry.run(&ctx));
    }

    let base_features = feature_summaries_by_path(&base_docs);
    Ok(Some(
        run_cost_analysis(
            cost_config,
            project.dbt.as_ref(),
            &inputs,
            &mut base_diagnostics,
            &base_features,
        )
        .summary,
    ))
}

fn evaluate_analysis(
    config: &ScanConfig,
    metrics: &crate::ScanMetrics,
    target_skipped_files: usize,
    metadata_only: bool,
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
            message: "a dbt manifest is required for this analysis policy".into(),
            observed: 0.0,
            allowed: 1.0,
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
            "run dbt compile and pass --manifest target/manifest.json for richer metadata",
            "no_manifest",
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
    let subject = path.to_string_lossy().replace('\\', "/");
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
                path: path.to_string_lossy().replace('\\', "/"),
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
            .map(|p| {
                p.strip_prefix(root)
                    .unwrap_or(p)
                    .to_string_lossy()
                    .replace('\\', "/")
            })
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
