use anyhow::{Context, Result};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DbtSqlFeatures {
    pub refs: Vec<DbtRef>,
    pub sources: Vec<DbtSourceRef>,
    pub config: DbtConfig,
    pub uses_is_incremental: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DbtConfig {
    pub materialized: Option<String>,
    pub unique_key: Option<String>,
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
    pub name: String,
    pub path: Option<PathBuf>,
    pub materialized: Option<String>,
    pub unique_key: Option<String>,
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

    let ref_re = Regex::new(r#"ref\s*\(\s*['"]([^'"]+)['"]\s*\)"#).expect("valid regex");
    for capture in ref_re.captures_iter(text) {
        features.refs.push(DbtRef {
            name: capture[1].to_string(),
        });
    }

    let source_re = Regex::new(r#"source\s*\(\s*['"]([^'"]+)['"]\s*,\s*['"]([^'"]+)['"]\s*\)"#)
        .expect("valid regex");
    for capture in source_re.captures_iter(text) {
        features.sources.push(DbtSourceRef {
            source_name: capture[1].to_string(),
            table_name: capture[2].to_string(),
        });
    }

    let config_re = Regex::new(r#"config\s*\((?s:.*?)\)"#).expect("valid regex");
    if let Some(capture) = config_re.find(text) {
        let body = capture.as_str();
        features.config.materialized = extract_config_value(body, "materialized");
        features.config.unique_key = extract_config_value(body, "unique_key");
    }

    features.uses_is_incremental = text.contains("is_incremental()");
    features
}

fn extract_config_value(body: &str, key: &str) -> Option<String> {
    let pattern = format!(r#"{key}\s*=\s*['"]([^'"]+)['"]"#);
    let regex = Regex::new(&pattern).expect("valid config regex");
    regex
        .captures(body)
        .and_then(|capture| capture.get(1).map(|value| value.as_str().to_string()))
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
                    name,
                    path,
                    materialized,
                    unique_key,
                    tags,
                    columns,
                    tests: Vec::new(),
                    refs,
                    sources,
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
}
