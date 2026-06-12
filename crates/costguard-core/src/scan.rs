use crate::baseline::{
    apply_finding_baseline, load_finding_baseline, write_finding_baseline, FindingBaseline,
};
use crate::config::ScanConfig;
use crate::dbt_graph::enrich_pr_summary;
use crate::dbt_load::load_dbt_project;
use crate::sql_analysis::{
    analyze_sql_documents, build_file_parse_status, build_scan_metrics, dbt_models_by_path,
    parse_failure_diagnostics,
};
use crate::{PrSummary, Project, ScanResult};
use anyhow::{Context, Result};
use costguard_cost::{annotate_diagnostics, summarize_features, CostInputs, ModelFeatureSummary};
use costguard_dbt::{compiled_code_by_model_path, MetadataWarning, MetadataWarningKind};
use costguard_diagnostics::{apply_suppressions, Diagnostic, Severity};
use costguard_rules::{ProjectIndexes, RuleMetadata, RuleRegistry};
use costguard_scanner::{
    discover_with_options, read_existing_paths_with_options, DiscoveryOptions, FileKind,
    ProjectFile, ScanCounts, SkippedFile,
};
use costguard_sql::{JoinKind, SqlDocument};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub fn scan(config: &ScanConfig) -> Result<ScanResult> {
    let root = config
        .root
        .canonicalize()
        .with_context(|| format!("failed to resolve root {}", config.root.display()))?;
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
    let scan_sql_documents = if config.changed_only {
        analyze_sql_documents(&project.files, config.platform, &compiled_by_path)
    } else {
        context_sql_documents
            .iter()
            .filter(|doc| project.files.iter().any(|file| file.path == doc.path))
            .cloned()
            .collect()
    };
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
        let ctx = costguard_rules::RuleContext {
            warehouse: config.platform,
            file,
            sql,
            dbt_model,
            all_sql: &context_sql_documents,
            project_indexes: &project_indexes,
            overrides: &config.rule_overrides,
        };
        let file_diagnostics = apply_suppressions(&file.text, registry.run(&ctx));
        diagnostics.extend(file_diagnostics);
    }

    diagnostics.extend(parse_failure_diagnostics(&context_sql_documents, &root));

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
            let mut combined = existing.findings;
            for diagnostic in &diagnostics {
                combined.push(crate::baseline::BaselinedFinding {
                    fingerprint: crate::baseline::diagnostic_fingerprint(diagnostic),
                    rule_id: diagnostic.rule_id.clone(),
                    path: diagnostic.path.to_string_lossy().replace('\\', "/"),
                    message: Some(diagnostic.message.clone()),
                });
            }
            combined.sort_by(|left, right| {
                left.path
                    .cmp(&right.path)
                    .then(left.rule_id.cmp(&right.rule_id))
                    .then(left.fingerprint.cmp(&right.fingerprint))
            });
            combined.dedup_by(|left, right| left.fingerprint == right.fingerprint);
            FindingBaseline {
                version: 1,
                warehouse: Some(config.platform.to_string()),
                findings: combined,
            }
        } else {
            FindingBaseline::from_diagnostics(&diagnostics, Some(&config.platform.to_string()))
        };
        write_finding_baseline(&output, &baseline_to_write)?;
    }

    let baseline = if let Some(path) = &config.baseline_path {
        Some(load_finding_baseline(
            if path.is_absolute() {
                path.clone()
            } else {
                root.join(path)
            }
            .as_path(),
        )?)
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
            Some(annotate_diagnostics(
                &mut diagnostics,
                cost_config,
                project.dbt.as_ref(),
                &inputs,
                &root,
                &features_by_path,
            ))
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
    Ok(ScanResult {
        diagnostics,
        counts: metrics.counts.clone(),
        metrics,
        file_parse_status,
        pr_summary,
        cost_summary,
    })
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
