use crate::baseline::{
    apply_finding_baseline, load_finding_baseline, validate_finding_baseline,
    write_finding_baseline,
};
use crate::config::ScanConfig;
use crate::dbt_graph::enrich_pr_summary;
use crate::dbt_load::load_dbt_project;
use crate::sql_analysis::{
    analyze_sql_documents, build_file_parse_status, build_scan_metrics, dbt_models_by_path,
    parse_failure_diagnostics,
};
use crate::{
    AnalysisReport, AnalysisViolation, PolicyMetadata, PrSummary, Project, RunMetadata, ScanResult,
};
use anyhow::{Context, Result};
use costguard_cost::{run_cost_analysis, summarize_features, CostInputs, ModelFeatureSummary};
use costguard_dbt::{compiled_code_by_model_path, MetadataWarning, MetadataWarningKind};
use costguard_diagnostics::{apply_suppressions, Diagnostic, Severity};
use costguard_policy::{
    apply_governance, read_signed_policy, read_trust_store, resolve_policy, verify_policy,
    PolicyDocumentV1, ResolutionContext, ResolvedPolicy,
};
use costguard_rules::{ProjectIndexes, RuleMetadata, RuleOverride, RuleOverrides, RuleRegistry};
use costguard_scanner::{
    discover_with_options, read_existing_paths_with_options, DiscoveryOptions, FileKind,
    ProjectFile, ScanCounts, SkippedFile,
};
use costguard_sql::{JoinKind, SqlDocument};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub fn scan(config: &ScanConfig) -> Result<ScanResult> {
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
    let (files, context_files, pr_summary, skipped_files) = if config.changed_only {
        if !costguard_git::is_git_repository(&root) {
            anyhow::bail!("{} is not a git repository", root.display());
        }
        let base = config.base_branch.as_deref().unwrap_or("main");
        let changed_files = costguard_git::changed_files(&root, base)
            .with_context(|| format!("failed to resolve changed files against base '{base}'"))?;
        let changed_discovery =
            read_existing_paths_with_options(&root, &changed_files, &discovery_options)?;
        let context_discovery = discover_with_options(&root, &config.paths, &discovery_options)?;
        let context_files = context_discovery
            .files
            .into_iter()
            .filter(is_dbt_context_file)
            .collect::<Vec<_>>();
        let mut skipped_files = changed_discovery.skipped_files;
        skipped_files.extend(context_discovery.skipped_files);
        (
            changed_discovery.files,
            Some(context_files),
            Some(PrSummary {
                changed_files,
                ..PrSummary::default()
            }),
            skipped_files,
        )
    } else {
        let discovery = discover_with_options(&root, &config.paths, &discovery_options)?;
        (discovery.files, None, None, discovery.skipped_files)
    };

    let counts = ScanCounts::from_files(&files);
    let context_for_dbt = context_files.as_deref().unwrap_or(&files);
    let dbt_load = load_dbt_project(&root, config, context_for_dbt)?;
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
    let mut metadata_warnings = dbt_load.warnings;
    metadata_warnings.extend(file_skip_warnings(&skipped_files));
    let metadata_diagnostics = metadata_diagnostics(&root, &metadata_warnings);
    let dbt = dbt_load.project;
    let project = Project {
        root: root.clone(),
        files,
        dbt,
    };

    let compiled_by_path = project
        .dbt
        .as_ref()
        .map(compiled_code_by_model_path)
        .unwrap_or_default();
    let context_files_ref = context_files.as_deref().unwrap_or(&project.files);
    let context_sql_documents =
        analyze_sql_documents(context_files_ref, config.platform, &compiled_by_path);
    let project_indexes = ProjectIndexes::from_sql_documents(&context_sql_documents);
    let changed_sql_documents = if config.changed_only {
        Some(analyze_sql_documents(
            &project.files,
            config.platform,
            &compiled_by_path,
        ))
    } else {
        None
    };
    let scan_sql_documents = changed_sql_documents
        .as_deref()
        .unwrap_or(&context_sql_documents);
    let sql_by_path = scan_sql_documents
        .iter()
        .map(|doc| (doc.path.clone(), doc))
        .collect::<HashMap<_, _>>();
    let dbt_by_path = dbt_models_by_path(&project);
    let registry = RuleRegistry::default_rules();
    let mut diagnostics = metadata_diagnostics;

    for file in &project.files {
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
            all_sql: &context_sql_documents,
            project_indexes: &project_indexes,
            overrides: &overrides,
        };
        let mut file_diagnostics = registry.run(&ctx);
        if let Some(policy) = &resolved {
            file_diagnostics.extend(registry.run_declarative(&ctx, &policy.custom_rules)?);
        }
        if managed_policy
            .as_ref()
            .is_none_or(|policy| policy.document.permissions.allow_inline_suppressions)
        {
            file_diagnostics = apply_suppressions(&file.text, file_diagnostics);
        }
        diagnostics.extend(file_diagnostics);
    }

    diagnostics.extend(parse_failure_diagnostics(&context_sql_documents, &root));
    assign_finding_identities(&mut diagnostics);
    let policy_violations =
        apply_managed_governance(&mut diagnostics, managed_policy.as_ref(), started_at)?;
    let policy_digest = managed_policy
        .as_ref()
        .map(|policy| policy.digest.clone())
        .unwrap_or_else(|| "local-unmanaged".into());

    if let Some(path) = &config.write_baseline_path {
        let output = if path.is_absolute() {
            path.clone()
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
            validate_baseline_policy(&existing, &policy_digest)?;
            crate::baseline::merge_baseline_findings(
                &existing,
                &diagnostics,
                config.platform,
                Some(policy_digest.clone()),
            )
        } else {
            crate::FindingBaseline::from_diagnostics(
                &diagnostics,
                config.platform,
                Some(policy_digest.clone()),
            )
        };
        write_finding_baseline(&output, &baseline_to_write)?;
    }

    let baseline = if let Some(path) = &config.baseline_path {
        let baseline = load_finding_baseline(
            if path.is_absolute() {
                path.clone()
            } else {
                root.join(path)
            }
            .as_path(),
        )?;
        validate_finding_baseline(&baseline, config.platform)?;
        validate_baseline_policy(&baseline, &policy_digest)?;
        Some(baseline)
    } else {
        None
    };

    let (mut diagnostics, baselined_findings) = if let Some(baseline) = &baseline {
        apply_finding_baseline(diagnostics, baseline)
    } else {
        (diagnostics, 0)
    };

    let features_by_path = feature_summaries_by_path(&context_sql_documents);
    let cost_summary = if let Some(cost_config) = &config.cost {
        if cost_config.enabled {
            let inputs = CostInputs::load(&root, cost_config)?;
            Some(
                run_cost_analysis(
                    cost_config,
                    project.dbt.as_ref(),
                    &inputs,
                    &mut diagnostics,
                    &features_by_path,
                )
                .summary,
            )
        } else {
            None
        }
    } else {
        None
    };

    diagnostics.sort_by(|left, right| {
        left.path
            .cmp(&right.path)
            .then(left.line.cmp(&right.line))
            .then(left.rule_id.cmp(&right.rule_id))
    });

    let pr_summary = pr_summary.map(|summary| enrich_pr_summary(summary, &project));
    let file_parse_status = build_file_parse_status(&context_sql_documents);
    let mut metrics = build_scan_metrics(
        &context_sql_documents,
        &diagnostics,
        counts,
        metadata_only,
        metadata_warnings.len(),
        yaml_parse_failures,
        dbt_project_parse_failures,
    );
    metrics.baselined_findings = baselined_findings;
    metrics.new_findings = diagnostics.len();
    let mut analysis = evaluate_analysis(config, &metrics, skipped_files.len(), metadata_only);
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
        file_parse_status,
        pr_summary,
        cost_summary,
        analysis,
    })
}

struct ManagedPolicy {
    document: PolicyDocumentV1,
    digest: String,
    organization: String,
    team: Option<String>,
    repository: String,
}

impl ManagedPolicy {
    fn resolve(&self, path: Option<&str>) -> Result<ResolvedPolicy> {
        resolve_policy(
            &self.document,
            &ResolutionContext {
                organization: &self.organization,
                team: self.team.as_deref(),
                repository: &self.repository,
                path,
            },
        )
    }

    fn metadata(&self) -> PolicyMetadata {
        PolicyMetadata {
            digest: self.digest.clone(),
            version: self.document.version.clone(),
            scope: "resolved-per-path".into(),
        }
    }
}

fn load_managed_policy(
    config: &ScanConfig,
    root: &Path,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<Option<ManagedPolicy>> {
    let Some(bundle_path) = &config.signed_policy.bundle_path else {
        return Ok(None);
    };
    let trust_path = config
        .signed_policy
        .trust_store_path
        .as_ref()
        .context("policy trust store is required")?;
    let resolve_path = |path: &Path| {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            root.join(path)
        }
    };
    let signed = read_signed_policy(&resolve_path(bundle_path))?;
    let trust = read_trust_store(&resolve_path(trust_path))?;
    let document = verify_policy(&signed, &trust, now)?;
    let organization = config
        .signed_policy
        .organization
        .clone()
        .unwrap_or_else(|| document.organization.clone());
    let repository = config
        .signed_policy
        .repository
        .clone()
        .or_else(|| std::env::var("GITHUB_REPOSITORY").ok())
        .or_else(|| {
            root.file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .context("unable to determine policy repository")?;
    let resolved = resolve_policy(
        &document,
        &ResolutionContext {
            organization: &organization,
            team: config.signed_policy.team.as_deref(),
            repository: &repository,
            path: None,
        },
    )?;
    Ok(Some(ManagedPolicy {
        document,
        digest: resolved.digest,
        organization,
        team: config.signed_policy.team.clone(),
        repository,
    }))
}

fn validate_local_policy_controls(
    config: &ScanConfig,
    policy: Option<&ManagedPolicy>,
) -> Result<()> {
    let Some(policy) = policy else {
        return Ok(());
    };
    let permissions = &policy.document.permissions;
    if (config.baseline_path.is_some() || config.write_baseline_path.is_some())
        && !permissions.allow_repository_baselines
    {
        anyhow::bail!("organization policy forbids repository baselines");
    }
    if !config.rule_overrides.is_empty()
        && !permissions.allow_local_severity_overrides
        && !permissions.allow_cli_overrides
    {
        anyhow::bail!("organization policy forbids local rule overrides");
    }
    Ok(())
}

fn effective_rule_overrides(config: &ScanConfig, policy: Option<&ResolvedPolicy>) -> RuleOverrides {
    let mut overrides = if policy.is_none()
        || policy.is_some_and(|policy| {
            policy.document.permissions.allow_local_severity_overrides
                || policy.document.permissions.allow_cli_overrides
        }) {
        config.rule_overrides.clone()
    } else {
        RuleOverrides::default()
    };
    if let Some(policy) = policy {
        for (rule_id, settings) in &policy.rules {
            let entry = overrides
                .entry(rule_id.clone())
                .or_insert_with(RuleOverride::default);
            if let Some(enabled) = settings.enabled {
                entry.enabled = Some(enabled);
            }
            if let Some(severity) = settings.severity {
                entry.severity = Some(severity);
            }
            if let Some(threshold) = settings.threshold {
                entry.threshold = Some(threshold);
            }
        }
    }
    overrides
}

fn apply_managed_governance(
    diagnostics: &mut [Diagnostic],
    managed: Option<&ManagedPolicy>,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<Vec<costguard_policy::PolicyViolation>> {
    let Some(managed) = managed else {
        return Ok(Vec::new());
    };
    let mut violations = Vec::new();
    for diagnostic in diagnostics {
        let path = normalized_path(&diagnostic.path);
        let resolved = managed.resolve(Some(&path))?;
        violations.extend(apply_governance(
            std::slice::from_mut(diagnostic),
            &resolved,
            &managed.repository,
            now,
        )?);
    }
    violations.sort_by(|left, right| {
        left.code
            .cmp(&right.code)
            .then(left.message.cmp(&right.message))
    });
    violations.dedup_by(|left, right| left.code == right.code && left.message == right.message);
    Ok(violations)
}

fn normalized_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
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

fn assign_finding_identities(diagnostics: &mut [Diagnostic]) {
    diagnostics.sort_by(|left, right| {
        left.path
            .cmp(&right.path)
            .then(left.rule_id.cmp(&right.rule_id))
            .then(left.line.cmp(&right.line))
            .then(left.column.cmp(&right.column))
    });
    let mut occurrences = std::collections::HashMap::<(PathBuf, String), usize>::new();
    for diagnostic in diagnostics {
        let key = (
            diagnostic.path.clone(),
            diagnostic.rule_id.to_ascii_uppercase(),
        );
        let ordinal = occurrences.entry(key).or_default();
        let evidence_key = if diagnostic.governance.evidence_key.is_empty() {
            format!("occurrence:{ordinal}")
        } else {
            diagnostic.governance.evidence_key.clone()
        };
        *ordinal += 1;
        diagnostic.assign_identity(evidence_key);
    }
}

fn evaluate_analysis(
    config: &ScanConfig,
    metrics: &crate::ScanMetrics,
    skipped_files: usize,
    metadata_only: bool,
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
    if skipped_files > analysis.max_skipped_files {
        violations.push(AnalysisViolation {
            code: "skipped_files".into(),
            message: format!(
                "{skipped_files} skipped or unreadable files exceed allowed {}",
                analysis.max_skipped_files
            ),
            observed: skipped_files as f64,
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

pub fn explain(config: &ScanConfig, path: &Path) -> Result<ScanResult> {
    let mut config = config.clone();
    config.paths = vec![path.to_path_buf()];
    scan(&config)
}

pub fn rules() -> Vec<RuleMetadata> {
    RuleRegistry::default_rules().metadata()
}

fn is_dbt_context_file(file: &ProjectFile) -> bool {
    matches!(
        file.kind,
        FileKind::DbtSqlModel | FileKind::DbtYaml | FileKind::ManifestJson
    )
}

fn file_skip_warnings(skipped_files: &[SkippedFile]) -> Vec<MetadataWarning> {
    skipped_files
        .iter()
        .map(|file| MetadataWarning {
            kind: MetadataWarningKind::FileSkipped,
            path: Some(file.path.clone()),
            message: format!(
                "skipped {}: {}",
                file.root_relative_path.display(),
                file.reason
            ),
        })
        .collect()
}

fn metadata_diagnostics(root: &Path, warnings: &[MetadataWarning]) -> Vec<Diagnostic> {
    warnings
        .iter()
        .map(|warning| metadata_warning_to_diagnostic(root, warning))
        .collect()
}

fn metadata_warning_to_diagnostic(root: &Path, warning: &MetadataWarning) -> Diagnostic {
    let (rule_id, severity, suggestion) = match warning.kind {
        MetadataWarningKind::NoManifest => (
            "SQLCOST023",
            Severity::Info,
            "run dbt compile and pass --manifest target/manifest.json for richer metadata",
        ),
        MetadataWarningKind::YamlParseFailed => (
            "SQLCOST024",
            Severity::Low,
            "fix the schema YAML syntax so Costguard can read model config",
        ),
        MetadataWarningKind::DbtProjectParseFailed
        | MetadataWarningKind::DbtProjectAmbiguousModels => (
            "SQLCOST025",
            Severity::Low,
            "fix dbt_project.yml syntax or project name alignment for folder config",
        ),
        MetadataWarningKind::FileSkipped => (
            "SQLCOST026",
            Severity::Low,
            "narrow scan paths, ignore generated files, or raise [scan].max_file_bytes in costguard.toml",
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
    Diagnostic::new(rule_id, severity, path, None, warning.message.clone())
        .with_suggestion(suggestion)
}
