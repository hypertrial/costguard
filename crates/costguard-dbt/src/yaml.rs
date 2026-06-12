use crate::project_config::parse_config_node;
use crate::{
    DbtColumn, DbtExposure, DbtModel, DbtProject, DbtSource, DbtTest, MetadataWarning,
    MetadataWarningKind,
};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub fn parse_yaml_project(text: &str) -> DbtProject {
    parse_yaml_project_with_warnings(text, Path::new("<yaml>")).0
}

pub fn parse_yaml_project_with_warnings(
    text: &str,
    path: &Path,
) -> (DbtProject, Vec<MetadataWarning>) {
    let Ok(value) = serde_yaml::from_str::<Value>(text) else {
        return (
            DbtProject::default(),
            vec![MetadataWarning {
                kind: MetadataWarningKind::YamlParseFailed,
                path: Some(path.to_path_buf()),
                message: format!("failed to parse schema YAML at {}", path.display()),
            }],
        );
    };
    (parse_yaml_value(value), Vec::new())
}

fn parse_yaml_value(value: Value) -> DbtProject {
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
                    partition_by: yaml_config.partition_by,
                    cluster_by: yaml_config.cluster_by,
                    full_refresh: yaml_config.full_refresh,
                    on_schema_change: yaml_config.on_schema_change,
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
        let matching_keys = target
            .models
            .iter()
            .filter_map(|(key, model)| (model.name == yaml_model.name).then_some(key.clone()))
            .collect::<Vec<_>>();
        if matching_keys.is_empty() {
            let key = synthesized_model_id(None, None, &name);
            target.models.insert(
                key,
                DbtModel {
                    unique_id: Some(synthesized_model_id(None, None, &name)),
                    name,
                    tags: yaml_model.tags.clone(),
                    tests: yaml_model.tests.clone(),
                    columns: yaml_model.columns.clone(),
                    materialized: yaml_model.materialized.clone(),
                    unique_key: yaml_model.unique_key.clone(),
                    incremental_strategy: yaml_model.incremental_strategy.clone(),
                    partition_by: yaml_model.partition_by.clone(),
                    cluster_by: yaml_model.cluster_by.clone(),
                    full_refresh: yaml_model.full_refresh,
                    on_schema_change: yaml_model.on_schema_change.clone(),
                    ..DbtModel::default()
                },
            );
            continue;
        }
        for key in matching_keys {
            let Some(model) = target.models.get_mut(&key) else {
                continue;
            };
            merge_model_metadata(model, &yaml_model);
        }
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

fn merge_model_metadata(model: &mut DbtModel, yaml_model: &DbtModel) {
    if model.tags.is_empty() {
        model.tags = yaml_model.tags.clone();
    } else {
        for tag in &yaml_model.tags {
            if !model.tags.contains(tag) {
                model.tags.push(tag.clone());
            }
        }
    }
    merge_model_config_fields(model, yaml_model);
    merge_columns(&mut model.columns, yaml_model.columns.clone());
    merge_tests(&mut model.tests, yaml_model.tests.clone());
}

pub fn synthesized_model_id(package_name: Option<&str>, path: Option<&Path>, name: &str) -> String {
    if let Some(path) = path {
        return format!(
            "model.local.{}",
            normalize_identifier(&path.to_string_lossy())
        );
    }
    let package = package_name.unwrap_or("local");
    format!("model.{package}.{}", normalize_identifier(name))
}

fn normalize_identifier(value: &str) -> String {
    value
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
        .collect()
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
    if target.partition_by.is_none() {
        target.partition_by = source.partition_by.clone();
    }
    if target.cluster_by.is_none() {
        target.cluster_by = source.cluster_by.clone();
    }
    if target.full_refresh.is_none() {
        target.full_refresh = source.full_refresh;
    }
    if target.on_schema_change.is_none() {
        target.on_schema_change = source.on_schema_change.clone();
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
