use crate::{DbtConfig, DbtModel};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default)]
pub struct FolderConfigMap {
    /// Folder path relative to the subproject `models/` directory (e.g. `marts`, `marts/finance`).
    pub configs: HashMap<String, DbtConfig>,
}

#[derive(Debug, Clone)]
pub struct DbtProjectFile {
    pub path: PathBuf,
    pub project_name: Option<String>,
    pub folder_configs: FolderConfigMap,
}

pub fn parse_config_node(value: &Value) -> DbtConfig {
    match value {
        Value::Object(map) => parse_config_object(map),
        _ => DbtConfig::default(),
    }
}

pub fn parse_config_object(map: &serde_json::Map<String, Value>) -> DbtConfig {
    let mut config = DbtConfig::default();

    for (key, value) in map {
        let normalized = normalize_config_key(key);
        match normalized.as_str() {
            "config" => merge_config(&mut config, &parse_config_node(value)),
            "materialized" => config.materialized = parse_string_value(value),
            "unique_key" => config.unique_key = parse_unique_key_value(value),
            "incremental_strategy" => config.incremental_strategy = parse_string_value(value),
            _ if key.starts_with('+') => {
                let stripped = key.trim_start_matches('+');
                match stripped {
                    "config" => merge_config(&mut config, &parse_config_node(value)),
                    "materialized" => config.materialized = parse_string_value(value),
                    "unique_key" => config.unique_key = parse_unique_key_value(value),
                    "incremental_strategy" => {
                        config.incremental_strategy = parse_string_value(value);
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    config
}

pub fn merge_config_fill_missing(target: &mut DbtConfig, source: &DbtConfig) {
    if target.materialized.is_none() {
        target.materialized = source.materialized.clone();
    }
    if target.unique_key.is_none() {
        target.unique_key = source.unique_key.clone();
    }
    if target.incremental_strategy.is_none() {
        target.incremental_strategy = source.incremental_strategy.clone();
    }
}

pub fn merge_config_override(mut target: DbtConfig, source: &DbtConfig) -> DbtConfig {
    if source.materialized.is_some() {
        target.materialized = source.materialized.clone();
    }
    if source.unique_key.is_some() {
        target.unique_key = source.unique_key.clone();
    }
    if source.incremental_strategy.is_some() {
        target.incremental_strategy = source.incremental_strategy.clone();
    }
    target
}

fn merge_config(target: &mut DbtConfig, source: &DbtConfig) {
    if source.materialized.is_some() {
        target.materialized = source.materialized.clone();
    }
    if source.unique_key.is_some() {
        target.unique_key = source.unique_key.clone();
    }
    if source.incremental_strategy.is_some() {
        target.incremental_strategy = source.incremental_strategy.clone();
    }
}

fn normalize_config_key(key: &str) -> String {
    key.trim_start_matches('+').to_string()
}

fn parse_string_value(value: &Value) -> Option<String> {
    value.as_str().map(str::to_string)
}

fn parse_unique_key_value(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.clone()),
        Value::Array(values) => {
            let parts = values.iter().filter_map(Value::as_str).collect::<Vec<_>>();
            if parts.is_empty() {
                None
            } else {
                Some(parts.join(","))
            }
        }
        _ => None,
    }
}

const CONFIG_KEYS: &[&str] = &[
    "config",
    "+config",
    "materialized",
    "+materialized",
    "unique_key",
    "+unique_key",
    "incremental_strategy",
    "+incremental_strategy",
    "tags",
    "+tags",
];

fn is_config_or_meta_key(key: &str) -> bool {
    CONFIG_KEYS.contains(&key)
        || key.starts_with('+')
        || key == "description"
        || key == "meta"
        || key == "docs"
        || key == "tests"
        || key == "columns"
        || key == "data_tests"
}

pub fn parse_dbt_project_text(text: &str) -> DbtProjectFile {
    let Ok(value) = serde_yaml::from_str::<Value>(text) else {
        return DbtProjectFile {
            path: PathBuf::new(),
            project_name: None,
            folder_configs: FolderConfigMap::default(),
        };
    };

    let project_name = value
        .get("name")
        .and_then(Value::as_str)
        .map(str::to_string);

    let mut folder_configs = FolderConfigMap::default();
    if let Some(models) = value.get("models") {
        if let Some(project_node) = resolve_project_models_node(models, project_name.as_deref()) {
            walk_folder_tree(
                project_node,
                "",
                DbtConfig::default(),
                &mut folder_configs.configs,
            );
        }
    }

    DbtProjectFile {
        path: PathBuf::new(),
        project_name,
        folder_configs,
    }
}

fn resolve_project_models_node<'a>(
    models: &'a Value,
    project_name: Option<&str>,
) -> Option<&'a Value> {
    let object = models.as_object()?;
    if let Some(name) = project_name {
        if let Some(node) = object.get(name) {
            return Some(node);
        }
    }
    if object.len() == 1 {
        return object.values().next();
    }
    None
}

fn walk_folder_tree(
    node: &Value,
    path_prefix: &str,
    inherited: DbtConfig,
    map: &mut HashMap<String, DbtConfig>,
) {
    let direct = node
        .as_object()
        .map(parse_config_object)
        .unwrap_or_default();
    let current = merge_config_override(inherited, &direct);
    map.insert(path_prefix.to_string(), current.clone());

    let Some(object) = node.as_object() else {
        return;
    };

    for (key, value) in object {
        if is_config_or_meta_key(key) {
            continue;
        }
        let child_prefix = if path_prefix.is_empty() {
            key.clone()
        } else {
            format!("{path_prefix}/{key}")
        };
        walk_folder_tree(value, &child_prefix, current.clone(), map);
    }
}

pub fn discover_dbt_project_files(root: &Path) -> Vec<DbtProjectFile> {
    let mut files = Vec::new();
    collect_dbt_project_files(root, root, &mut files);
    files.sort_by(|left, right| left.path.cmp(&right.path));
    files
}

#[allow(clippy::only_used_in_recursion)]
fn collect_dbt_project_files(root: &Path, current: &Path, files: &mut Vec<DbtProjectFile>) {
    let entry_path = current.join("dbt_project.yml");
    if entry_path.is_file() {
        if let Ok(text) = std::fs::read_to_string(&entry_path) {
            let mut project = parse_dbt_project_text(&text);
            project.path = entry_path.clone();
            files.push(project);
        }
    }

    let Ok(read_dir) = std::fs::read_dir(current) else {
        return;
    };

    for entry in read_dir.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.')
            || name == "target"
            || name == "dbt_packages"
            || name == "node_modules"
        {
            continue;
        }
        collect_dbt_project_files(root, &path, files);
    }
}

pub fn resolve_models_relative_path(model_path: &Path, project_file: &Path) -> Option<String> {
    let project_dir = project_file.parent()?;
    let models_dir = project_dir.join("models");
    let relative = model_path.strip_prefix(&models_dir).ok()?;
    let parent = relative.parent()?;
    if parent.as_os_str().is_empty() {
        Some(String::new())
    } else {
        Some(normalize_path(parent))
    }
}

pub fn find_dbt_project_for_model<'a>(
    model_path: &Path,
    project_files: &'a [DbtProjectFile],
) -> Option<&'a DbtProjectFile> {
    project_files
        .iter()
        .filter(|project| {
            project
                .path
                .parent()
                .is_some_and(|parent| model_path.starts_with(parent))
        })
        .max_by_key(|project| project.path.components().count())
}

pub fn resolve_folder_config_for_model(
    model_path: &Path,
    project_file: &DbtProjectFile,
) -> DbtConfig {
    let folder = resolve_models_relative_path(model_path, &project_file.path).unwrap_or_default();
    resolve_folder_config(&project_file.folder_configs, &folder)
}

pub fn resolve_folder_config(map: &FolderConfigMap, folder: &str) -> DbtConfig {
    let normalized = normalize_path(Path::new(folder));
    if let Some(config) = map.configs.get(&normalized) {
        return config.clone();
    }

    let mut config = map.configs.get("").cloned().unwrap_or_default();
    if normalized.is_empty() {
        return config;
    }

    let mut prefix = String::new();
    for segment in normalized.split('/') {
        prefix = if prefix.is_empty() {
            segment.to_string()
        } else {
            format!("{prefix}/{segment}")
        };
        if let Some(folder_config) = map.configs.get(&prefix) {
            config = folder_config.clone();
        }
    }

    config
}

pub fn apply_folder_config_to_model(model: &mut DbtModel, folder_config: &DbtConfig) {
    let mut config = DbtConfig {
        materialized: model.materialized.clone(),
        unique_key: model.unique_key.clone(),
        incremental_strategy: model.incremental_strategy.clone(),
    };
    merge_config_fill_missing(&mut config, folder_config);
    model.materialized = config.materialized;
    model.unique_key = config.unique_key;
    model.incremental_strategy = config.incremental_strategy;
}

fn normalize_path(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "/")
        .trim_matches('/')
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_model_config_block_in_schema_yaml() {
        let yaml = r#"
config:
  materialized: incremental
  unique_key: tx_hash
  incremental_strategy: merge
"#;
        let value: Value = serde_yaml::from_str(yaml).unwrap();
        let config = parse_config_node(value.get("config").unwrap());
        assert_eq!(config.materialized.as_deref(), Some("incremental"));
        assert_eq!(config.unique_key.as_deref(), Some("tx_hash"));
        assert_eq!(config.incremental_strategy.as_deref(), Some("merge"));
    }

    #[test]
    fn parses_unique_key_list() {
        let yaml = r#"unique_key: [col1, col2]"#;
        let value: Value = serde_yaml::from_str(yaml).unwrap();
        let config = parse_config_object(value.as_object().unwrap());
        assert_eq!(config.unique_key.as_deref(), Some("col1,col2"));
    }

    #[test]
    fn parses_nested_dbt_project_folder_configs() {
        let yaml = r#"
name: spellbook
models:
  spellbook:
    +materialized: view
    marts:
      +materialized: incremental
      +unique_key: tx_hash
"#;
        let project = parse_dbt_project_text(yaml);
        let marts = resolve_folder_config(&project.folder_configs, "marts");
        assert_eq!(marts.materialized.as_deref(), Some("incremental"));
        assert_eq!(marts.unique_key.as_deref(), Some("tx_hash"));
    }

    #[test]
    fn parses_plus_config_block_at_folder_level() {
        let yaml = r#"
name: pkg
models:
  pkg:
    marts:
      +config:
        materialized: incremental
        unique_key: id
"#;
        let project = parse_dbt_project_text(yaml);
        let marts = resolve_folder_config(&project.folder_configs, "marts");
        assert_eq!(marts.materialized.as_deref(), Some("incremental"));
        assert_eq!(marts.unique_key.as_deref(), Some("id"));
    }

    #[test]
    fn resolves_subproject_model_folder_path() {
        let project_path = PathBuf::from("/repo/dbt_subprojects/dex/dbt_project.yml");
        let model_path = PathBuf::from("/repo/dbt_subprojects/dex/models/marts/fct_trades.sql");
        let folder = resolve_models_relative_path(&model_path, &project_path).expect("folder path");
        assert_eq!(folder, "marts");
    }

    #[test]
    fn folder_config_cascades_from_parent_to_child() {
        let yaml = r#"
name: pkg
models:
  pkg:
    +incremental_strategy: append
    marts:
      +materialized: incremental
      finance:
        +unique_key: id
"#;
        let project = parse_dbt_project_text(yaml);
        let finance = resolve_folder_config(&project.folder_configs, "marts/finance");
        assert_eq!(finance.materialized.as_deref(), Some("incremental"));
        assert_eq!(finance.unique_key.as_deref(), Some("id"));
    }
}
