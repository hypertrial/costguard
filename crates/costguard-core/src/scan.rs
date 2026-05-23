use crate::config::ScanConfig;
use crate::dbt_graph::enrich_pr_summary;
use crate::{FileParseStatus, PrSummary, Project, ScanMetrics, ScanResult};
use anyhow::{Context, Result};
use costguard_dbt::{
    apply_dbt_project_configs_in_roots, compiled_code_by_model_path, extract_sql_features,
    merge_yaml_project, parse_manifest, parse_yaml_project_with_warnings, synthesized_model_id,
    DbtModel, DbtProject, MetadataWarning, MetadataWarningKind,
};
use costguard_diagnostics::{apply_suppressions, Diagnostic, Severity};
use costguard_platform::Platform;
use costguard_rules::{ProjectIndexes, RuleMetadata, RuleRegistry};
use costguard_scanner::{
    discover_with_options, read_existing_paths_with_options, DiscoveryOptions, FileKind,
    ProjectFile, ScanCounts, SkippedFile,
};
use costguard_sql::{analyze_sql, SqlDocument};
use rayon::prelude::*;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

struct DbtLoadOutcome {
    project: Option<DbtProject>,
    warnings: Vec<MetadataWarning>,
    metadata_only: bool,
}

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
        context_sql_documents.clone()
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

    diagnostics.sort_by(|left, right| {
        left.path
            .cmp(&right.path)
            .then(left.line.cmp(&right.line))
            .then(left.rule_id.cmp(&right.rule_id))
    });

    let pr_summary = pr_summary.map(|summary| enrich_pr_summary(summary, &project));
    let file_parse_status = build_file_parse_status(&context_sql_documents);
    let metrics = build_scan_metrics(
        &context_sql_documents,
        &diagnostics,
        counts,
        metadata_only,
        metadata_warnings.len(),
        yaml_parse_failures,
        dbt_project_parse_failures,
    );
    Ok(ScanResult {
        diagnostics,
        counts: metrics.counts.clone(),
        metrics,
        file_parse_status,
        pr_summary,
    })
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

fn load_dbt_project(
    root: &Path,
    config: &ScanConfig,
    files: &[ProjectFile],
) -> Result<DbtLoadOutcome> {
    let mut warnings = Vec::new();
    let explicit_manifest_path = config.manifest_path.as_ref().map(|path| {
        if path.is_absolute() {
            path.clone()
        } else {
            root.join(path)
        }
    });
    if let Some(path) = &explicit_manifest_path {
        if !path.exists() {
            anyhow::bail!("manifest path does not exist: {}", path.display());
        }
    }

    let manifest_path = explicit_manifest_path
        .clone()
        .or_else(|| {
            let auto = root.join("target/manifest.json");
            auto.exists().then_some(auto)
        })
        .or_else(|| {
            files
                .iter()
                .find(|file| file.kind == FileKind::ManifestJson)
                .map(|file| file.path.clone())
        });

    let loaded_manifest = manifest_path.is_some();
    let mut project = match manifest_path {
        Some(path) => parse_manifest(&path)?,
        None => DbtProject::default(),
    };

    let has_dbt_files = files.iter().any(|file| {
        matches!(
            file.kind,
            FileKind::DbtSqlModel | FileKind::DbtYaml | FileKind::ManifestJson
        )
    });
    if !loaded_manifest && has_dbt_files {
        warnings.push(MetadataWarning {
            kind: MetadataWarningKind::NoManifest,
            path: None,
            message: "no dbt manifest found; using YAML and SQL metadata only".to_string(),
        });
    }

    for file in files {
        if file.kind == FileKind::DbtYaml {
            let (yaml_project, yaml_warnings) =
                parse_yaml_project_with_warnings(&file.text, &file.path);
            merge_yaml_project(&mut project, yaml_project);
            warnings.extend(yaml_warnings);
        }
    }

    for file in files {
        if file.kind != FileKind::DbtSqlModel {
            continue;
        }
        let features = extract_sql_features(&file.text);
        let name = file
            .path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or_default()
            .to_string();
        let model_key = find_model_key_for_file(&project, &file.root_relative_path, &name)
            .unwrap_or_else(|| synthesized_model_id(None, Some(&file.root_relative_path), &name));
        let model = project
            .models
            .entry(model_key.clone())
            .or_insert_with(|| DbtModel {
                unique_id: Some(model_key.clone()),
                node_id: Some(model_key.clone()),
                name: name.clone(),
                path: Some(file.root_relative_path.clone()),
                ..DbtModel::default()
            });
        model.path = Some(file.root_relative_path.clone());
        model.materialized = model
            .materialized
            .clone()
            .or(features.config.materialized.clone());
        model.unique_key = model
            .unique_key
            .clone()
            .or(features.config.unique_key.clone());
        model.incremental_strategy = model
            .incremental_strategy
            .clone()
            .or(features.config.incremental_strategy.clone());
        if model.refs.is_empty() {
            model.refs = features
                .refs
                .into_iter()
                .map(|reference| match reference.package {
                    Some(package) => format!("{package}.{}", reference.name),
                    None => reference.name,
                })
                .collect();
        }
        if model.sources.is_empty() {
            model.sources = features.sources;
        }
        let dependency_key = model.identity().to_string();
        let dependencies = model.refs.clone();
        project
            .graph
            .depends_on
            .entry(dependency_key)
            .or_insert(dependencies);
    }

    warnings.extend(apply_dbt_project_configs_in_roots(
        root,
        &scan_roots(root, &config.paths),
        &mut project,
    ));

    let metadata_only = !loaded_manifest && has_dbt_files;
    let project = if project.models.is_empty() {
        None
    } else {
        Some(project)
    };

    Ok(DbtLoadOutcome {
        project,
        warnings,
        metadata_only,
    })
}

fn scan_roots(root: &Path, paths: &[PathBuf]) -> Vec<PathBuf> {
    if paths.is_empty() {
        return vec![root.to_path_buf()];
    }
    let mut roots = paths
        .iter()
        .map(|path| {
            if path.is_absolute() {
                path.clone()
            } else {
                root.join(path)
            }
        })
        .flat_map(|path| {
            let mut roots = vec![path.clone()];
            if path.file_name().and_then(|name| name.to_str()) == Some("models") {
                if let Some(parent) = path.parent() {
                    roots.push(parent.to_path_buf());
                }
            }
            roots
        })
        .collect::<Vec<_>>();
    roots.sort();
    roots.dedup();
    roots
}

fn find_model_key_for_file(project: &DbtProject, path: &Path, name: &str) -> Option<String> {
    if let Some((key, _)) = project
        .models
        .iter()
        .find(|(_, model)| model.path.as_deref() == Some(path))
    {
        return Some(key.clone());
    }
    let matches = project
        .models
        .iter()
        .filter_map(|(key, model)| (model.name == name).then_some(key.clone()))
        .collect::<Vec<_>>();
    (matches.len() == 1).then(|| matches[0].clone())
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

fn analyze_sql_documents(
    files: &[ProjectFile],
    platform: Platform,
    compiled_by_path: &HashMap<PathBuf, String>,
) -> Vec<SqlDocument> {
    let mut documents = files
        .par_iter()
        .filter(|file| matches!(file.kind, FileKind::Sql | FileKind::DbtSqlModel))
        .map(|file| {
            let compiled_code = compiled_by_path
                .get(&file.root_relative_path)
                .map(String::as_str);
            analyze_sql(
                file.path.clone(),
                &file.text,
                platform,
                &file.line_index,
                compiled_code,
                file.kind == FileKind::DbtSqlModel,
            )
        })
        .collect::<Vec<_>>();
    documents.sort_by(|left, right| left.path.cmp(&right.path));
    documents
}

fn build_file_parse_status(sql_documents: &[SqlDocument]) -> Vec<FileParseStatus> {
    sql_documents
        .iter()
        .filter(|doc| doc.is_dbt_sql_model)
        .map(|doc| FileParseStatus {
            path: doc.path.clone(),
            parse_input: doc.parse_input,
            parsed: doc.parsed,
            parsed_raw: doc.parsed_raw,
            parsed_compiled: doc.parsed_compiled,
            feature_extraction_used_ast: doc.feature_extraction_used_ast,
        })
        .collect()
}

fn build_scan_metrics(
    sql_documents: &[SqlDocument],
    diagnostics: &[costguard_diagnostics::Diagnostic],
    counts: ScanCounts,
    metadata_only: bool,
    metadata_warnings: usize,
    yaml_parse_failures: usize,
    dbt_project_parse_failures: usize,
) -> ScanMetrics {
    let model_docs = sql_documents
        .iter()
        .filter(|doc| doc.is_dbt_sql_model)
        .collect::<Vec<_>>();
    let other_docs = sql_documents
        .iter()
        .filter(|doc| !doc.is_dbt_sql_model)
        .collect::<Vec<_>>();
    let compiled_docs = model_docs
        .iter()
        .filter(|doc| doc.used_compiled_for_parse)
        .copied()
        .collect::<Vec<_>>();

    let sql_parse_total = model_docs.len();
    let sql_parse_failures = model_docs.iter().filter(|doc| !doc.parsed).count();
    let sql_parse_other_total = other_docs.len();
    let sql_parse_other_failures = other_docs.iter().filter(|doc| !doc.parsed).count();
    let sql_parse_compiled_total = compiled_docs.len();
    let sql_parse_compiled_failures = compiled_docs
        .iter()
        .filter(|doc| !doc.parsed_compiled)
        .count();
    let mut diagnostics_by_rule = BTreeMap::new();
    let mut diagnostics_by_severity = BTreeMap::new();
    for diagnostic in diagnostics {
        *diagnostics_by_rule
            .entry(diagnostic.rule_id.clone())
            .or_default() += 1;
        *diagnostics_by_severity
            .entry(diagnostic.severity.label().to_ascii_lowercase())
            .or_default() += 1;
    }
    ScanMetrics {
        counts,
        sql_parse_total,
        sql_parse_failures,
        sql_parse_other_total,
        sql_parse_other_failures,
        sql_parse_compiled_total,
        sql_parse_compiled_failures,
        metadata_warnings,
        yaml_parse_failures,
        dbt_project_parse_failures,
        metadata_only_scan: metadata_only,
        diagnostics_by_rule,
        diagnostics_by_severity,
    }
}

fn dbt_models_by_path(project: &Project) -> HashMap<PathBuf, &DbtModel> {
    project
        .dbt
        .as_ref()
        .into_iter()
        .flat_map(|dbt| dbt.models.values())
        .filter_map(|model| model.path.as_ref().map(|path| (path.clone(), model)))
        .collect()
}
