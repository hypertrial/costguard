use crate::config::ScanConfig;
use crate::dbt_graph::enrich_pr_summary;
use crate::{FileParseStatus, PrSummary, Project, ScanMetrics, ScanResult};
use anyhow::{Context, Result};
use costguard_dbt::{
    apply_dbt_project_configs, compiled_code_by_model_path, extract_sql_features,
    merge_yaml_project, parse_manifest, parse_yaml_project_with_warnings, DbtModel, DbtProject,
    MetadataWarning, MetadataWarningKind,
};
use costguard_diagnostics::{apply_suppressions, Diagnostic, Severity};
use costguard_platform::Platform;
use costguard_rules::{ProjectIndexes, RuleMetadata, RuleRegistry};
use costguard_scanner::{discover, read_existing_paths, FileKind, ProjectFile, ScanCounts};
use costguard_sql::{analyze_sql, SqlDocument};
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
    let ignored = ignored_paths(&root, &config.ignore);
    let (mut files, mut context_files, pr_summary) = if config.changed_only {
        if !costguard_git::is_git_repository(&root) {
            anyhow::bail!("{} is not a git repository", root.display());
        }
        let base = config.base_branch.as_deref().unwrap_or("main");
        let changed_files = costguard_git::changed_files(&root, base)
            .with_context(|| format!("failed to resolve changed files against base '{base}'"))?;
        let files = read_existing_paths(&root, &changed_files)?;
        let context_files = discover(&root, &config.paths)?
            .into_iter()
            .filter(is_dbt_context_file)
            .collect::<Vec<_>>();
        (
            files,
            Some(context_files),
            Some(PrSummary {
                changed_files,
                ..PrSummary::default()
            }),
        )
    } else {
        let files = discover(&root, &config.paths)?;
        (files, None, None)
    };

    if !ignored.is_empty() {
        files.retain(|file| !is_ignored(file, &ignored));
        if let Some(context_files) = context_files.as_mut() {
            context_files.retain(|file| !is_ignored(file, &ignored));
        }
    }

    let counts = ScanCounts::from_files(&files);
    let context_for_dbt = context_files.as_deref().unwrap_or(&files);
    let dbt_load = load_dbt_project(&root, config, context_for_dbt)?;
    let metadata_only = dbt_load.metadata_only;
    let metadata_warnings_count = dbt_load.warnings.len();
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
    let metadata_diagnostics = metadata_diagnostics(&root, &dbt_load.warnings);
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
        metadata_warnings_count,
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

fn ignored_paths(root: &Path, ignore: &[PathBuf]) -> Vec<PathBuf> {
    ignore
        .iter()
        .map(|path| {
            if path.is_absolute() {
                path.clone()
            } else {
                root.join(path)
            }
        })
        .collect()
}

fn is_ignored(file: &ProjectFile, ignored: &[PathBuf]) -> bool {
    ignored
        .iter()
        .any(|ignored_path| file.path.starts_with(ignored_path))
}

fn is_dbt_context_file(file: &ProjectFile) -> bool {
    matches!(
        file.kind,
        FileKind::DbtSqlModel | FileKind::DbtYaml | FileKind::ManifestJson
    )
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
        let model = project
            .models
            .entry(name.clone())
            .or_insert_with(|| DbtModel {
                name,
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
                .map(|reference| reference.name)
                .collect();
        }
        if model.sources.is_empty() {
            model.sources = features.sources;
        }
        let dependency_key = model.node_id.clone().unwrap_or_else(|| model.name.clone());
        let dependencies = model.refs.clone();
        project
            .graph
            .depends_on
            .entry(dependency_key)
            .or_insert(dependencies);
    }

    warnings.extend(apply_dbt_project_configs(root, &mut project));

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
    files
        .iter()
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
        .collect()
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
