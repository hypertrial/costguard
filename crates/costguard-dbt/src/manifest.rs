use crate::{DbtColumn, DbtExposure, DbtModel, DbtProject, DbtSource, DbtSourceRef};
use anyhow::{Context, Result};
use serde_json::Value;
use std::path::{Path, PathBuf};

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
            let package_name = node
                .get("package_name")
                .and_then(Value::as_str)
                .map(str::to_string);
            let fqn = node
                .get("fqn")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>();
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
            let partition_by = config
                .get("partition_by")
                .and_then(parse_config_value_string);
            let cluster_by = config.get("cluster_by").and_then(parse_config_value_string);
            let full_refresh = config.get("full_refresh").and_then(Value::as_bool);
            let on_schema_change = config
                .get("on_schema_change")
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
            let owners = metadata_owners(node, config);
            let group = node
                .get("group")
                .or_else(|| config.get("group"))
                .and_then(Value::as_str)
                .map(str::to_string);
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
                node_id.clone(),
                DbtModel {
                    unique_id: Some(node_id.clone()),
                    node_id: Some(node_id.clone()),
                    package_name,
                    fqn,
                    name,
                    path,
                    materialized,
                    unique_key,
                    incremental_strategy,
                    partition_by,
                    cluster_by,
                    full_refresh,
                    on_schema_change,
                    compiled_code,
                    tags,
                    owners,
                    group,
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
            let mut owners = owners_from_value(exposure.get("owner"));
            append_owners(&mut owners, exposure.get("meta"));
            append_owners(
                &mut owners,
                exposure.get("config").and_then(|config| config.get("meta")),
            );
            project.exposures.insert(
                name.to_string(),
                DbtExposure {
                    name: name.to_string(),
                    depends_on,
                    owners,
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

fn metadata_owners(node: &Value, config: &Value) -> Vec<String> {
    let mut owners = Vec::new();
    append_owners(&mut owners, node.get("meta"));
    append_owners(&mut owners, config.get("meta"));
    owners
}

fn append_owners(owners: &mut Vec<String>, meta: Option<&Value>) {
    let Some(meta) = meta else { return };
    for value in [meta.get("owner"), meta.get("owners")] {
        for owner in owners_from_value(value) {
            if !owners.contains(&owner) {
                owners.push(owner);
            }
        }
    }
}

fn owners_from_value(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::String(owner)) => vec![owner.clone()],
        Some(Value::Array(owners)) => owners
            .iter()
            .flat_map(|owner| owners_from_value(Some(owner)))
            .collect(),
        Some(Value::Object(owner)) => [owner.get("name"), owner.get("email")]
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect(),
        _ => Vec::new(),
    }
}

fn parse_config_value_string(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::String(s) => Some(s.clone()),
        Value::Bool(b) => Some(b.to_string()),
        Value::Number(n) => Some(n.to_string()),
        other => Some(other.to_string()),
    }
}
