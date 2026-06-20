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
use std::time::SystemTime;

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

// ponytail: mtime heuristic when manifest nodes lack sha256 checksums; checksum
// comparison is preferred when dbt wrote checksum.name = "sha256".
fn manifest_is_stale(
    manifest_mtime: SystemTime,
    mut source_mtimes: impl Iterator<Item = SystemTime>,
) -> bool {
    source_mtimes.any(|mtime| mtime > manifest_mtime)
}

fn detect_stale_manifest(
    manifest_path: &Path,
    files: &[ProjectFile],
    project: &DbtProject,
) -> Option<MetadataWarning> {
    let manifest_mtime = std::fs::metadata(manifest_path).ok()?.modified().ok()?;
    let models_by_path = project
        .models
        .values()
        .filter_map(|model| model.path.as_ref().map(|path| (path.clone(), model)))
        .collect::<HashMap<_, _>>();
    let mut source_mtimes = Vec::new();
    let mut newer_count = 0usize;
    for file in files {
        if file.kind != FileKind::DbtSqlModel {
            continue;
        }
        if models_by_path
            .get(&file.root_relative_path)
            .is_some_and(|model| model_has_sha256_checksum(model))
        {
            continue;
        }
        let Some(source_mtime) = std::fs::metadata(&file.path)
            .ok()
            .and_then(|meta| meta.modified().ok())
        else {
            continue;
        };
        source_mtimes.push(source_mtime);
        if source_mtime > manifest_mtime {
            newer_count += 1;
        }
    }
    if !manifest_is_stale(manifest_mtime, source_mtimes.into_iter()) {
        return None;
    }
    Some(MetadataWarning {
        kind: MetadataWarningKind::StaleManifest,
        path: Some(manifest_path.to_path_buf()),
        message: format!(
            "dbt manifest is older than {newer_count} modified model file(s); compiled SQL may be stale"
        ),
    })
}

pub(crate) fn detect_checksum_mismatches(
    project: &DbtProject,
    files: &[ProjectFile],
    changed_paths: &std::collections::HashSet<PathBuf>,
) -> Vec<MetadataWarning> {
    let models_by_path = project
        .models
        .values()
        .filter_map(|model| model.path.as_ref().map(|path| (path.clone(), model)))
        .collect::<HashMap<_, _>>();
    let mut warnings = Vec::new();
    for file in files {
        if file.kind != FileKind::DbtSqlModel {
            continue;
        }
        if !changed_paths.contains(&file.root_relative_path) {
            continue;
        }
        let Some(model) = models_by_path.get(&file.root_relative_path) else {
            continue;
        };
        if !model_has_sha256_checksum(model) {
            continue;
        }
        let expected = model.checksum.as_deref().expect("checksum checked");
        let actual = costguard_diagnostics::hex_sha256(file.text.as_bytes());
        if actual == expected {
            continue;
        }
        warnings.push(MetadataWarning {
            kind: MetadataWarningKind::ManifestChecksumMismatch,
            path: Some(file.path.clone()),
            message: format!(
                "model {} source checksum does not match manifest (head file may be newer than dbt compile)",
                file.root_relative_path.display()
            ),
        });
    }
    warnings
}

fn model_has_sha256_checksum(model: &costguard_dbt::DbtModel) -> bool {
    model
        .checksum
        .as_ref()
        .is_some_and(|value| !value.is_empty())
        && model
            .checksum_kind
            .as_deref()
            .is_some_and(|kind| kind.eq_ignore_ascii_case("sha256"))
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
    let mut project = match manifest_path.as_ref() {
        Some(path) => parse_manifest(path)?,
        None => DbtProject::default(),
    };

    if let Some(path) = manifest_path.as_ref() {
        if let Some(warning) = detect_stale_manifest(path, files, &project) {
            warnings.push(warning);
        }
    }
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

    #[test]
    fn manifest_is_stale_when_any_source_is_newer() {
        use std::time::{Duration, UNIX_EPOCH};
        let manifest = UNIX_EPOCH + Duration::from_secs(100);
        let older = UNIX_EPOCH + Duration::from_secs(50);
        let newer = UNIX_EPOCH + Duration::from_secs(150);
        assert!(!manifest_is_stale(manifest, [older, manifest].into_iter()));
        assert!(manifest_is_stale(manifest, [older, newer].into_iter()));
    }

    #[test]
    fn detect_checksum_mismatch_when_file_content_differs() {
        let text = "select 1";
        let checksum = costguard_diagnostics::hex_sha256(text.as_bytes());
        let mut project = DbtProject::default();
        project.models.insert(
            "model.pkg.orders".into(),
            DbtModel {
                unique_id: Some("model.pkg.orders".into()),
                name: "orders".into(),
                path: Some(PathBuf::from("models/orders.sql")),
                checksum: Some(checksum),
                checksum_kind: Some("sha256".into()),
                ..DbtModel::default()
            },
        );
        let files = vec![ProjectFile {
            path: PathBuf::from("/repo/models/orders.sql"),
            root_relative_path: PathBuf::from("models/orders.sql"),
            kind: FileKind::DbtSqlModel,
            line_index: costguard_diagnostics::LineIndex::new("select 2"),
            text: "select 2".into(),
        }];
        let changed = HashSet::from([PathBuf::from("models/orders.sql")]);
        let warnings = detect_checksum_mismatches(&project, &files, &changed);
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].kind, MetadataWarningKind::ManifestChecksumMismatch);

        let matching = detect_checksum_mismatches(
            &project,
            &[ProjectFile {
                text: text.into(),
                ..files[0].clone()
            }],
            &changed,
        );
        assert!(matching.is_empty());
    }
}
