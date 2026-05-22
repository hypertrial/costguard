use crate::config::ScanConfig;
use crate::dbt_graph::enrich_pr_summary;
use crate::{PrSummary, Project, ScanMetrics, ScanResult};
use anyhow::{Context, Result};
use costguard_dbt::{
    apply_dbt_project_configs, extract_sql_features, merge_yaml_project, parse_manifest,
    parse_yaml_project, DbtModel, DbtProject,
};
use costguard_diagnostics::apply_suppressions;
use costguard_platform::Platform;
use costguard_rules::{ProjectIndexes, RuleMetadata, RuleRegistry};
use costguard_scanner::{discover, read_existing_paths, FileKind, ProjectFile, ScanCounts};
use costguard_sql::{analyze_sql, SqlDocument};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

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
    let dbt = load_dbt_project(&root, config, context_files.as_deref().unwrap_or(&files))?;
    let project = Project {
        root: root.clone(),
        files,
        dbt,
    };

    let context_files_ref = context_files.as_deref().unwrap_or(&project.files);
    let context_sql_documents = analyze_sql_documents(context_files_ref, config.platform);
    let project_indexes = ProjectIndexes::from_sql_documents(&context_sql_documents);
    let scan_sql_documents = if config.changed_only {
        analyze_sql_documents(&project.files, config.platform)
    } else {
        context_sql_documents.clone()
    };
    let sql_by_path = scan_sql_documents
        .iter()
        .map(|doc| (doc.path.clone(), doc))
        .collect::<HashMap<_, _>>();
    let dbt_by_path = dbt_models_by_path(&project);
    let registry = RuleRegistry::default_rules();
    let mut diagnostics = Vec::new();

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
    let metrics = build_scan_metrics(&context_sql_documents, &diagnostics, counts);
    Ok(ScanResult {
        diagnostics,
        counts: metrics.counts.clone(),
        metrics,
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
) -> Result<Option<DbtProject>> {
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

    let mut project = match manifest_path {
        Some(path) => parse_manifest(&path)?,
        None => DbtProject::default(),
    };

    for file in files {
        if file.kind == FileKind::DbtYaml {
            merge_yaml_project(&mut project, parse_yaml_project(&file.text));
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

    apply_dbt_project_configs(root, &mut project);

    if project.models.is_empty() {
        Ok(None)
    } else {
        Ok(Some(project))
    }
}

fn analyze_sql_documents(files: &[ProjectFile], platform: Platform) -> Vec<SqlDocument> {
    files
        .iter()
        .filter(|file| matches!(file.kind, FileKind::Sql | FileKind::DbtSqlModel))
        .map(|file| analyze_sql(file.path.clone(), &file.text, platform, &file.line_index))
        .collect()
}

fn build_scan_metrics(
    sql_documents: &[SqlDocument],
    diagnostics: &[costguard_diagnostics::Diagnostic],
    counts: ScanCounts,
) -> ScanMetrics {
    let sql_parse_total = sql_documents.len();
    let sql_parse_failures = sql_documents.iter().filter(|doc| !doc.parsed).count();
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
