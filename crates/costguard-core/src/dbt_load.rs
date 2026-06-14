use crate::config::ScanConfig;
use anyhow::Result;
use costguard_dbt::{
    apply_dbt_project_configs_in_roots, extract_sql_features, merge_yaml_project, parse_manifest,
    parse_yaml_project_with_warnings, synthesized_model_id, DbtModel, DbtProject, MetadataWarning,
    MetadataWarningKind,
};
use costguard_scanner::{FileKind, ProjectFile};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

pub(crate) struct DbtLoadOutcome {
    pub project: Option<DbtProject>,
    pub warnings: Vec<MetadataWarning>,
    pub metadata_only: bool,
}

#[derive(Default)]
struct ModelIndex {
    by_path: HashMap<PathBuf, String>,
    by_name: HashMap<String, HashSet<String>>,
}

impl ModelIndex {
    fn from_project(project: &DbtProject) -> Self {
        let mut index = Self::default();
        for (key, model) in &project.models {
            index.insert(key, model.path.as_deref(), &model.name);
        }
        index
    }

    fn find(&self, path: &Path, name: &str) -> Option<String> {
        self.by_path.get(path).cloned().or_else(|| {
            let keys = self.by_name.get(name)?;
            (keys.len() == 1).then(|| keys.iter().next().expect("one model key").clone())
        })
    }

    fn update(
        &mut self,
        key: &str,
        previous_path: Option<&Path>,
        previous_name: Option<&str>,
        path: Option<&Path>,
        name: &str,
    ) {
        if let Some(previous_path) = previous_path {
            if self
                .by_path
                .get(previous_path)
                .is_some_and(|value| value == key)
            {
                self.by_path.remove(previous_path);
            }
        }
        if let Some(previous_name) = previous_name {
            if let Some(keys) = self.by_name.get_mut(previous_name) {
                keys.remove(key);
                if keys.is_empty() {
                    self.by_name.remove(previous_name);
                }
            }
        }
        self.insert(key, path, name);
    }

    fn insert(&mut self, key: &str, path: Option<&Path>, name: &str) {
        if let Some(path) = path {
            self.by_path.insert(path.to_path_buf(), key.to_string());
        }
        self.by_name
            .entry(name.to_string())
            .or_default()
            .insert(key.to_string());
    }
}

fn model_key_for_file(index: &ModelIndex, path: &Path, name: &str) -> String {
    index
        .find(path, name)
        .unwrap_or_else(|| synthesized_model_id(None, Some(path), name))
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

    let mut model_index = ModelIndex::from_project(&project);

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
        let model_key = model_key_for_file(&model_index, &file.root_relative_path, &name);
        let previous = project
            .models
            .get(&model_key)
            .map(|model| (model.path.clone(), model.name.clone()));
        let (dependency_key, dependencies) = {
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
            (model.identity().to_string(), model.refs.clone())
        };
        let model = project.models.get(&model_key).expect("model was inserted");
        model_index.update(
            &model_key,
            previous.as_ref().and_then(|(path, _)| path.as_deref()),
            previous.as_ref().map(|(_, name)| name.as_str()),
            model.path.as_deref(),
            &model.name,
        );
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

#[cfg(test)]
mod tests {
    use super::*;

    fn model(name: &str, path: Option<&str>) -> DbtModel {
        DbtModel {
            name: name.to_string(),
            path: path.map(PathBuf::from),
            ..DbtModel::default()
        }
    }

    #[test]
    fn model_index_prefers_exact_path() {
        let project = DbtProject {
            models: HashMap::from([
                (
                    "model.pkg.orders".into(),
                    model("orders", Some("models/orders.sql")),
                ),
                (
                    "model.other.orders".into(),
                    model("orders", Some("other/orders.sql")),
                ),
            ]),
            ..DbtProject::default()
        };
        let index = ModelIndex::from_project(&project);

        assert_eq!(
            index
                .find(Path::new("models/orders.sql"), "orders")
                .as_deref(),
            Some("model.pkg.orders")
        );
    }

    #[test]
    fn model_index_uses_unique_name_fallback() {
        let project = DbtProject {
            models: HashMap::from([(
                "model.pkg.orders".into(),
                model("orders", Some("legacy/orders.sql")),
            )]),
            ..DbtProject::default()
        };
        let index = ModelIndex::from_project(&project);

        assert_eq!(
            index
                .find(Path::new("models/orders.sql"), "orders")
                .as_deref(),
            Some("model.pkg.orders")
        );
    }

    #[test]
    fn model_index_rejects_ambiguous_name_fallback() {
        let project = DbtProject {
            models: HashMap::from([
                ("model.pkg.orders".into(), model("orders", None)),
                ("model.other.orders".into(), model("orders", None)),
            ]),
            ..DbtProject::default()
        };
        let index = ModelIndex::from_project(&project);

        assert_eq!(index.find(Path::new("models/orders.sql"), "orders"), None);
    }

    #[test]
    fn model_key_is_synthesized_when_lookup_misses() {
        let path = Path::new("models/orders.sql");
        let key = model_key_for_file(&ModelIndex::default(), path, "orders");

        assert_eq!(key, synthesized_model_id(None, Some(path), "orders"));
    }

    #[test]
    fn model_index_updates_reassigned_path() {
        let project = DbtProject {
            models: HashMap::from([(
                "model.pkg.orders".into(),
                model("orders", Some("legacy/orders.sql")),
            )]),
            ..DbtProject::default()
        };
        let mut index = ModelIndex::from_project(&project);
        index.update(
            "model.pkg.orders",
            Some(Path::new("legacy/orders.sql")),
            Some("orders"),
            Some(Path::new("models/orders.sql")),
            "orders",
        );

        assert!(!index.by_path.contains_key(Path::new("legacy/orders.sql")));
        assert_eq!(
            index
                .by_path
                .get(Path::new("models/orders.sql"))
                .map(String::as_str),
            Some("model.pkg.orders")
        );
    }
}
