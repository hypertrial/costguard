mod project_config;

use anyhow::{Context, Result};
use project_config::{
    apply_folder_config_to_model, discover_dbt_project_files, find_dbt_project_for_model,
    parse_config_node, resolve_folder_config_for_model,
};
pub use project_config::{parse_config_node as parse_dbt_config, DbtProjectFile, FolderConfigMap};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DbtSqlFeatures {
    pub refs: Vec<DbtRef>,
    pub sources: Vec<DbtSourceRef>,
    pub config: DbtConfig,
    pub uses_is_incremental: bool,
    pub incremental_block: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DbtConfig {
    pub materialized: Option<String>,
    pub unique_key: Option<String>,
    pub incremental_strategy: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DbtRef {
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DbtSourceRef {
    pub source_name: String,
    pub table_name: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DbtProject {
    pub models: HashMap<String, DbtModel>,
    pub sources: HashMap<String, DbtSource>,
    pub exposures: HashMap<String, DbtExposure>,
    pub graph: DbtGraph,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DbtGraph {
    pub depends_on: HashMap<String, Vec<String>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DbtModel {
    pub node_id: Option<String>,
    pub name: String,
    pub path: Option<PathBuf>,
    pub materialized: Option<String>,
    pub unique_key: Option<String>,
    pub incremental_strategy: Option<String>,
    pub compiled_code: Option<String>,
    pub tags: Vec<String>,
    pub columns: Vec<DbtColumn>,
    pub tests: Vec<DbtTest>,
    pub refs: Vec<String>,
    pub sources: Vec<DbtSourceRef>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DbtSource {
    pub name: String,
    pub tables: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DbtExposure {
    pub name: String,
    pub depends_on: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DbtColumn {
    pub name: String,
    pub tests: Vec<DbtTest>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DbtTest {
    pub name: String,
    pub args: Value,
}

pub fn extract_sql_features(text: &str) -> DbtSqlFeatures {
    let mut features = DbtSqlFeatures::default();

    for capture in ref_regex().captures_iter(text) {
        features.refs.push(DbtRef {
            name: capture[1].to_string(),
        });
    }

    for capture in source_regex().captures_iter(text) {
        features.sources.push(DbtSourceRef {
            source_name: capture[1].to_string(),
            table_name: capture[2].to_string(),
        });
    }

    if let Some(capture) = config_regex().find(text) {
        let body = capture.as_str();
        features.config.materialized = extract_config_value(body, "materialized");
        features.config.unique_key = extract_config_value(body, "unique_key");
        features.config.incremental_strategy = extract_config_value(body, "incremental_strategy");
    }

    features.uses_is_incremental = text.contains("is_incremental()");
    features.incremental_block = extract_incremental_block(text);
    features
}

pub fn extract_incremental_block(text: &str) -> Option<String> {
    incremental_block_regex()
        .captures(text)
        .and_then(|capture| capture.get(1))
        .map(|matched| matched.as_str().trim().to_string())
        .filter(|block| !block.is_empty())
}

fn incremental_block_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?is)\{%\s*(?:if|elif)\s+is_incremental\s*\(\)\s*%\}(.*?)\{%\s*endif\s*%\}")
            .expect("valid incremental regex")
    })
}

fn extract_config_value(body: &str, key: &str) -> Option<String> {
    config_key_regex(key)
        .captures(body)
        .and_then(|capture| capture.get(1).map(|value| value.as_str().to_string()))
}

fn config_key_regex(key: &str) -> &'static Regex {
    static MATERIALIZED: OnceLock<Regex> = OnceLock::new();
    static UNIQUE_KEY: OnceLock<Regex> = OnceLock::new();
    static INCREMENTAL_STRATEGY: OnceLock<Regex> = OnceLock::new();
    match key {
        "materialized" => MATERIALIZED.get_or_init(|| {
            Regex::new(r#"materialized\s*=\s*['"]([^'"]+)['"]"#).expect("valid config regex")
        }),
        "unique_key" => UNIQUE_KEY.get_or_init(|| {
            Regex::new(r#"unique_key\s*=\s*['"]([^'"]+)['"]"#).expect("valid config regex")
        }),
        "incremental_strategy" => INCREMENTAL_STRATEGY.get_or_init(|| {
            Regex::new(r#"incremental_strategy\s*=\s*['"]([^'"]+)['"]"#)
                .expect("valid config regex")
        }),
        other => panic!("unsupported config key: {other}"),
    }
}

fn ref_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r#"ref\s*\(\s*['"]([^'"]+)['"]\s*\)"#).expect("valid regex"))
}

fn source_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r#"source\s*\(\s*['"]([^'"]+)['"]\s*,\s*['"]([^'"]+)['"]\s*\)"#)
            .expect("valid regex")
    })
}

fn config_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r#"config\s*\((?s:.*?)\)"#).expect("valid regex"))
}

pub fn parse_manifest(path: &Path) -> Result<DbtProject> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read manifest {}", path.display()))?;
    parse_manifest_text(&text)
}

pub fn parse_manifest_text(text: &str) -> Result<DbtProject> {
    let manifest: Value = serde_json::from_str(text).context("invalid dbt manifest JSON")?;
    let mut project = DbtProject::default();

    if let Some(nodes) = manifest.get("nodes").and_then(Value::as_object) {
        for (node_id, node) in nodes {
            if node.get("resource_type").and_then(Value::as_str) != Some("model") {
                continue;
            }
            let name = node
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or(node_id)
                .to_string();
            let path = node
                .get("original_file_path")
                .or_else(|| node.get("path"))
                .and_then(Value::as_str)
                .map(PathBuf::from);
            let config = node.get("config").unwrap_or(&Value::Null);
            let materialized = config
                .get("materialized")
                .and_then(Value::as_str)
                .map(str::to_string);
            let unique_key = config.get("unique_key").and_then(|value| match value {
                Value::String(s) => Some(s.clone()),
                Value::Array(values) => Some(
                    values
                        .iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join(","),
                ),
                _ => None,
            });
            let incremental_strategy = config
                .get("incremental_strategy")
                .and_then(Value::as_str)
                .map(str::to_string);
            let compiled_code = node
                .get("compiled_code")
                .or_else(|| node.get("compiled_sql"))
                .and_then(Value::as_str)
                .map(str::to_string)
                .filter(|code| !code.trim().is_empty());
            let refs = node
                .get("refs")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_array)
                .filter_map(|items| items.last().and_then(Value::as_str))
                .map(str::to_string)
                .collect();
            let sources = node
                .get("sources")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_array)
                .filter_map(|items| {
                    Some(DbtSourceRef {
                        source_name: items.first()?.as_str()?.to_string(),
                        table_name: items.get(1)?.as_str()?.to_string(),
                    })
                })
                .collect();
            let tags = node
                .get("tags")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect();
            let columns = node
                .get("columns")
                .and_then(Value::as_object)
                .into_iter()
                .flatten()
                .map(|(name, _)| DbtColumn {
                    name: name.clone(),
                    tests: Vec::new(),
                })
                .collect();
            let depends_on = node
                .get("depends_on")
                .and_then(|depends| depends.get("nodes"))
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>();
            project.graph.depends_on.insert(node_id.clone(), depends_on);
            project.models.insert(
                name.clone(),
                DbtModel {
                    node_id: Some(node_id.clone()),
                    name,
                    path,
                    materialized,
                    unique_key,
                    incremental_strategy,
                    compiled_code,
                    tags,
                    columns,
                    tests: Vec::new(),
                    refs,
                    sources,
                },
            );
        }
    }

    if let Some(exposures) = manifest.get("exposures").and_then(Value::as_object) {
        for exposure in exposures.values() {
            let Some(name) = exposure.get("name").and_then(Value::as_str) else {
                continue;
            };
            let depends_on = exposure
                .get("depends_on")
                .and_then(|depends| depends.get("nodes"))
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect();
            project.exposures.insert(
                name.to_string(),
                DbtExposure {
                    name: name.to_string(),
                    depends_on,
                },
            );
        }
    }

    if let Some(sources) = manifest.get("sources").and_then(Value::as_object) {
        for source in sources.values() {
            let source_name = source
                .get("source_name")
                .or_else(|| source.get("name"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let table_name = source
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            project
                .sources
                .entry(source_name.clone())
                .or_insert_with(|| DbtSource {
                    name: source_name,
                    tables: Vec::new(),
                })
                .tables
                .push(table_name);
        }
    }

    Ok(project)
}

pub fn parse_yaml_models(text: &str) -> Vec<DbtModel> {
    let Ok(value) = serde_yaml::from_str::<Value>(text) else {
        return Vec::new();
    };
    value
        .get("models")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|model| {
            let name = model.get("name")?.as_str()?.to_string();
            let columns = model
                .get("columns")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|column| {
                    Some(DbtColumn {
                        name: column.get("name")?.as_str()?.to_string(),
                        tests: Vec::new(),
                    })
                })
                .collect();
            Some(DbtModel {
                name,
                columns,
                ..DbtModel::default()
            })
        })
        .collect()
}

pub fn parse_yaml_project(text: &str) -> DbtProject {
    let Ok(value) = serde_yaml::from_str::<Value>(text) else {
        return DbtProject::default();
    };
    let mut project = DbtProject::default();

    if let Some(models) = value.get("models").and_then(Value::as_array) {
        for model in models {
            let Some(name) = model.get("name").and_then(Value::as_str) else {
                continue;
            };
            let tags = string_or_string_array(model.get("tags"));
            let tests = parse_tests(model.get("tests"));
            let yaml_config = model
                .get("config")
                .map(parse_config_node)
                .unwrap_or_default();
            let columns = model
                .get("columns")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(parse_column)
                .collect();
            project.models.insert(
                name.to_string(),
                DbtModel {
                    name: name.to_string(),
                    materialized: yaml_config.materialized,
                    unique_key: yaml_config.unique_key,
                    incremental_strategy: yaml_config.incremental_strategy,
                    tags,
                    tests,
                    columns,
                    ..DbtModel::default()
                },
            );
        }
    }

    if let Some(sources) = value.get("sources").and_then(Value::as_array) {
        for source in sources {
            let Some(name) = source.get("name").and_then(Value::as_str) else {
                continue;
            };
            let tables = source
                .get("tables")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|table| table.get("name").and_then(Value::as_str))
                .map(str::to_string)
                .collect();
            project.sources.insert(
                name.to_string(),
                DbtSource {
                    name: name.to_string(),
                    tables,
                },
            );
        }
    }

    if let Some(exposures) = value.get("exposures").and_then(Value::as_array) {
        for exposure in exposures {
            let Some(name) = exposure.get("name").and_then(Value::as_str) else {
                continue;
            };
            let depends_on = exposure
                .get("depends_on")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect();
            project.exposures.insert(
                name.to_string(),
                DbtExposure {
                    name: name.to_string(),
                    depends_on,
                },
            );
        }
    }

    project
}

pub fn merge_yaml_project(target: &mut DbtProject, yaml: DbtProject) {
    for (name, yaml_model) in yaml.models {
        let model = target
            .models
            .entry(name.clone())
            .or_insert_with(|| DbtModel {
                name,
                ..DbtModel::default()
            });
        if model.tags.is_empty() {
            model.tags = yaml_model.tags.clone();
        } else {
            for tag in &yaml_model.tags {
                if !model.tags.contains(tag) {
                    model.tags.push(tag.clone());
                }
            }
        }
        merge_model_config_fields(model, &yaml_model);
        merge_columns(&mut model.columns, yaml_model.columns);
        merge_tests(&mut model.tests, yaml_model.tests);
    }
    for (name, yaml_source) in yaml.sources {
        let source = target
            .sources
            .entry(name.clone())
            .or_insert_with(|| DbtSource {
                name,
                tables: Vec::new(),
            });
        for table in yaml_source.tables {
            if !source.tables.contains(&table) {
                source.tables.push(table);
            }
        }
    }
    for (name, exposure) in yaml.exposures {
        target.exposures.entry(name).or_insert(exposure);
    }
}

fn merge_model_config_fields(target: &mut DbtModel, source: &DbtModel) {
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

pub fn compiled_code_by_model_path(project: &DbtProject) -> HashMap<PathBuf, String> {
    project
        .models
        .values()
        .filter_map(|model| {
            let path = model.path.as_ref()?;
            let code = model.compiled_code.as_ref()?;
            Some((path.clone(), code.clone()))
        })
        .collect()
}

pub fn apply_dbt_project_configs(root: &Path, project: &mut DbtProject) {
    let project_files = discover_dbt_project_files(root);
    for model in project.models.values_mut() {
        let Some(model_path) = model.path.as_ref() else {
            continue;
        };
        let absolute_model_path = root.join(model_path);
        let Some(project_file) = find_dbt_project_for_model(&absolute_model_path, &project_files)
        else {
            continue;
        };
        let folder_config = resolve_folder_config_for_model(&absolute_model_path, project_file);
        apply_folder_config_to_model(model, &folder_config);
    }
}

fn merge_columns(target: &mut Vec<DbtColumn>, yaml_columns: Vec<DbtColumn>) {
    for yaml_column in yaml_columns {
        if let Some(column) = target
            .iter_mut()
            .find(|column| column.name == yaml_column.name)
        {
            merge_tests(&mut column.tests, yaml_column.tests);
        } else {
            target.push(yaml_column);
        }
    }
}

fn merge_tests(target: &mut Vec<DbtTest>, yaml_tests: Vec<DbtTest>) {
    for test in yaml_tests {
        if !target
            .iter()
            .any(|existing| existing.name == test.name && existing.args == test.args)
        {
            target.push(test);
        }
    }
}

fn parse_column(value: &Value) -> Option<DbtColumn> {
    Some(DbtColumn {
        name: value.get("name")?.as_str()?.to_string(),
        tests: parse_tests(value.get("tests")),
    })
}

fn parse_tests(value: Option<&Value>) -> Vec<DbtTest> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|test| match test {
            Value::String(name) => Some(DbtTest {
                name: name.clone(),
                args: Value::Null,
            }),
            Value::Object(map) => map.iter().next().map(|(name, args)| DbtTest {
                name: name.clone(),
                args: args.clone(),
            }),
            _ => None,
        })
        .collect()
}

fn string_or_string_array(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::String(tag)) => vec![tag.clone()],
        Some(Value::Array(tags)) => tags
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_dbt_sql_features() {
        let text = "{{ config(materialized='incremental', unique_key=\"id\") }} select * from {{ ref('stg_orders') }} where {% if is_incremental() %} updated_at > current_date {% endif %}";
        let features = extract_sql_features(text);
        assert_eq!(features.refs[0].name, "stg_orders");
        assert_eq!(features.config.materialized.as_deref(), Some("incremental"));
        assert_eq!(features.config.unique_key.as_deref(), Some("id"));
        assert!(features.uses_is_incremental);
    }

    #[test]
    fn extracts_incremental_block() {
        let text = r#"
select id from t
{% if is_incremental() %}
where id not in (select id from {{ this }})
{% endif %}
"#;
        let features = extract_sql_features(text);
        assert!(features.uses_is_incremental);
        let block = features.incremental_block.expect("incremental block");
        assert!(block.contains("not in (select id from {{ this }})"));
    }

    #[test]
    fn parses_yaml_models_sources_and_exposures() {
        let yaml = r#"
models:
  - name: fct_sessions
    tags: [mart, critical]
    tests:
      - unique:
          column_name: id
    columns:
      - name: id
        tests:
          - not_null
sources:
  - name: raw
    tables:
      - name: events
exposures:
  - name: dashboard
    depends_on:
      - ref('fct_sessions')
"#;
        let project = parse_yaml_project(yaml);
        let model = project.models.get("fct_sessions").unwrap();
        assert_eq!(model.tags, vec!["mart", "critical"]);
        assert_eq!(model.columns[0].tests[0].name, "not_null");
        assert_eq!(project.sources["raw"].tables, vec!["events"]);
        assert_eq!(
            project.exposures["dashboard"].depends_on[0],
            "ref('fct_sessions')"
        );
    }

    #[test]
    fn parses_yaml_model_config() {
        let yaml = r#"
models:
  - name: fct_sessions
    config:
      materialized: incremental
      unique_key: id
      incremental_strategy: merge
"#;
        let project = parse_yaml_project(yaml);
        let model = project.models.get("fct_sessions").unwrap();
        assert_eq!(model.materialized.as_deref(), Some("incremental"));
        assert_eq!(model.unique_key.as_deref(), Some("id"));
        assert_eq!(model.incremental_strategy.as_deref(), Some("merge"));
    }

    #[test]
    fn yaml_merge_enriches_model_config_fields() {
        let mut target = DbtProject::default();
        target.models.insert(
            "fct_sessions".into(),
            DbtModel {
                node_id: Some("model.pkg.fct_sessions".into()),
                name: "fct_sessions".into(),
                materialized: Some("incremental".into()),
                ..DbtModel::default()
            },
        );
        let yaml = parse_yaml_project(
            r#"
models:
  - name: fct_sessions
    config:
      unique_key: id
      incremental_strategy: merge
"#,
        );

        merge_yaml_project(&mut target, yaml);
        let model = target.models.get("fct_sessions").unwrap();
        assert_eq!(model.materialized.as_deref(), Some("incremental"));
        assert_eq!(model.unique_key.as_deref(), Some("id"));
        assert_eq!(model.incremental_strategy.as_deref(), Some("merge"));
    }

    #[test]
    fn yaml_merge_enriches_manifest_columns_and_tests() {
        let mut target = DbtProject::default();
        target.models.insert(
            "fct_sessions".into(),
            DbtModel {
                node_id: Some("model.pkg.fct_sessions".into()),
                name: "fct_sessions".into(),
                materialized: Some("incremental".into()),
                columns: vec![DbtColumn {
                    name: "id".into(),
                    tests: Vec::new(),
                }],
                ..DbtModel::default()
            },
        );
        let yaml = parse_yaml_project(
            r#"
models:
  - name: fct_sessions
    tags: [critical]
    tests:
      - unique:
          column_name: id
    columns:
      - name: id
        tests:
          - not_null
      - name: session_id
        tests:
          - not_null
"#,
        );

        merge_yaml_project(&mut target, yaml);
        let model = target.models.get("fct_sessions").unwrap();
        assert_eq!(model.materialized.as_deref(), Some("incremental"));
        assert_eq!(model.tags, vec!["critical"]);
        assert_eq!(model.tests[0].name, "unique");
        assert_eq!(model.columns.len(), 2);
        assert_eq!(model.columns[0].tests[0].name, "not_null");
    }

    #[test]
    fn parses_manifest_compiled_code_and_path_map() {
        let manifest = r#"{
  "nodes": {
    "model.pkg.fct_block_time": {
      "resource_type": "model",
      "name": "fct_block_time",
      "original_file_path": "models/marts/fct_block_time.sql",
      "config": { "materialized": "incremental" },
      "compiled_code": "select tx_hash, block_time from dex.trades"
    },
    "model.pkg.legacy_model": {
      "resource_type": "model",
      "name": "legacy_model",
      "original_file_path": "models/marts/legacy_model.sql",
      "config": { "materialized": "view" },
      "compiled_sql": "select 1 as id"
    }
  }
}"#;
        let project = parse_manifest_text(manifest).expect("parse manifest");
        let compiled = project.models.get("fct_block_time").unwrap();
        assert_eq!(
            compiled.compiled_code.as_deref(),
            Some("select tx_hash, block_time from dex.trades")
        );
        let legacy = project.models.get("legacy_model").unwrap();
        assert_eq!(legacy.compiled_code.as_deref(), Some("select 1 as id"));

        let by_path = compiled_code_by_model_path(&project);
        assert_eq!(by_path.len(), 2);
        assert_eq!(
            by_path
                .get(&PathBuf::from("models/marts/fct_block_time.sql"))
                .map(String::as_str),
            Some("select tx_hash, block_time from dex.trades")
        );
    }
}
