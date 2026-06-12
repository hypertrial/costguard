use crate::config::ScanConfig;
use anyhow::Result;
use costguard_dbt::{
    apply_dbt_project_configs_in_roots, extract_sql_features, merge_yaml_project, parse_manifest,
    parse_yaml_project_with_warnings, synthesized_model_id, DbtModel, DbtProject, MetadataWarning,
    MetadataWarningKind,
};
use costguard_scanner::{FileKind, ProjectFile};
use std::path::{Path, PathBuf};

pub(crate) struct DbtLoadOutcome {
    pub project: Option<DbtProject>,
    pub warnings: Vec<MetadataWarning>,
    pub metadata_only: bool,
}

pub(crate) fn load_dbt_project(
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

pub(crate) fn scan_roots(root: &Path, paths: &[PathBuf]) -> Vec<PathBuf> {
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

pub(crate) fn find_model_key_for_file(
    project: &DbtProject,
    path: &Path,
    name: &str,
) -> Option<String> {
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
