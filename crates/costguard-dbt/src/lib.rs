mod project_config;

use anyhow::{Context, Result};
use project_config::{
    apply_folder_config_to_model, discover_dbt_project_files_in_roots_with_warnings,
    find_dbt_project_for_model, parse_config_node, resolve_folder_config_for_model,
};
pub use project_config::{
    parse_config_node as parse_dbt_config, parse_dbt_project_with_warnings, DbtProjectFile,
    FolderConfigMap,
};
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
    pub package: Option<String>,
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

impl DbtProject {
    pub fn model_by_name(&self, name: &str) -> Option<&DbtModel> {
        let mut matches = self.models.values().filter(|model| model.name == name);
        let first = matches.next()?;
        matches.next().is_none().then_some(first)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DbtGraph {
    pub depends_on: HashMap<String, Vec<String>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DbtModel {
    pub unique_id: Option<String>,
    pub node_id: Option<String>,
    pub package_name: Option<String>,
    pub fqn: Vec<String>,
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

impl DbtModel {
    pub fn identity(&self) -> &str {
        self.unique_id
            .as_deref()
            .or(self.node_id.as_deref())
            .unwrap_or(&self.name)
    }
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MetadataWarningKind {
    YamlParseFailed,
    DbtProjectParseFailed,
    DbtProjectAmbiguousModels,
    NoManifest,
    FileSkipped,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetadataWarning {
    pub kind: MetadataWarningKind,
    pub path: Option<PathBuf>,
    pub message: String,
}

pub fn extract_sql_features(text: &str) -> DbtSqlFeatures {
    let mut features = DbtSqlFeatures::default();

    for capture in ref_regex().captures_iter(text) {
        let (package, name) = match capture.get(2) {
            Some(name) => (Some(capture[1].to_string()), name.as_str().to_string()),
            None => (None, capture[1].to_string()),
        };
        features.refs.push(DbtRef { package, name });
    }

    for capture in source_regex().captures_iter(text) {
        features.sources.push(DbtSourceRef {
            source_name: capture[1].to_string(),
            table_name: capture[2].to_string(),
        });
    }

    apply_inline_configs(text, &mut features.config);

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

fn apply_inline_configs(text: &str, config: &mut DbtConfig) {
    for body in config_call_bodies(text) {
        for item in split_top_level_commas(body) {
            let Some((key, value)) = split_top_level_assignment(item) else {
                continue;
            };
            match key.trim() {
                "materialized" => {
                    if let Some(value) = quoted_string(value.trim()) {
                        config.materialized = Some(value);
                    }
                }
                "unique_key" => {
                    if let Some(value) = parse_unique_key_literal(value.trim()) {
                        config.unique_key = Some(value);
                    }
                }
                "incremental_strategy" => {
                    if let Some(value) = quoted_string(value.trim()) {
                        config.incremental_strategy = Some(value);
                    }
                }
                _ => {}
            }
        }
    }
}

fn config_call_bodies(text: &str) -> Vec<&str> {
    let lower = text.to_ascii_lowercase();
    let mut bodies = Vec::new();
    let mut index = 0usize;
    while index < text.len() {
        if lower[index..].starts_with("config") && is_identifier_boundary(text, index) {
            let mut open = index + "config".len();
            while open < text.len() {
                let ch = text[open..].chars().next().unwrap_or_default();
                if !ch.is_whitespace() {
                    break;
                }
                open += ch.len_utf8();
            }
            if text[open..].starts_with('(') {
                if let Some(close) = balanced_config_close(text, open) {
                    bodies.push(&text[open + 1..close]);
                    index = close + 1;
                    continue;
                }
            }
        }
        let ch = text[index..].chars().next().unwrap_or_default();
        if ch == '\0' {
            break;
        }
        index += ch.len_utf8();
    }
    bodies
}

fn is_identifier_boundary(text: &str, index: usize) -> bool {
    text[..index]
        .chars()
        .next_back()
        .is_none_or(|ch| !is_identifier_char(ch))
}

fn is_identifier_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

fn balanced_config_close(text: &str, open: usize) -> Option<usize> {
    let mut paren_depth = 1usize;
    let mut bracket_depth = 0usize;
    let mut brace_depth = 0usize;
    let mut quote: Option<char> = None;
    let mut index = open + 1;
    while index < text.len() {
        let ch = text[index..].chars().next()?;
        if let Some(quote_ch) = quote {
            if ch == '\\' {
                index += ch.len_utf8();
                if index < text.len() {
                    index += text[index..].chars().next()?.len_utf8();
                }
                continue;
            }
            if ch == quote_ch {
                let next = index + ch.len_utf8();
                if text[next..].starts_with(quote_ch) {
                    index = next + ch.len_utf8();
                    continue;
                }
                quote = None;
            }
            index += ch.len_utf8();
            continue;
        }

        match ch {
            '\'' | '"' => quote = Some(ch),
            '(' => paren_depth += 1,
            ')' if bracket_depth == 0 && brace_depth == 0 => {
                paren_depth = paren_depth.saturating_sub(1);
                if paren_depth == 0 {
                    return Some(index);
                }
            }
            ')' => paren_depth = paren_depth.saturating_sub(1),
            '[' => bracket_depth += 1,
            ']' => bracket_depth = bracket_depth.saturating_sub(1),
            '{' => brace_depth += 1,
            '}' => brace_depth = brace_depth.saturating_sub(1),
            _ => {}
        }
        index += ch.len_utf8();
    }
    None
}

fn split_top_level_commas(text: &str) -> Vec<&str> {
    split_top_level(text, ',')
}

fn split_top_level_assignment(text: &str) -> Option<(&str, &str)> {
    let parts = split_top_level(text, '=');
    match parts.as_slice() {
        [key, value] => Some((*key, *value)),
        _ => None,
    }
}

fn split_top_level(text: &str, delimiter: char) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0usize;
    let mut paren_depth = 0usize;
    let mut bracket_depth = 0usize;
    let mut brace_depth = 0usize;
    let mut quote: Option<char> = None;
    let mut index = 0usize;
    while index < text.len() {
        let ch = text[index..].chars().next().unwrap_or_default();
        if let Some(quote_ch) = quote {
            if ch == '\\' {
                index += ch.len_utf8();
                if index < text.len() {
                    index += text[index..].chars().next().unwrap_or_default().len_utf8();
                }
                continue;
            }
            if ch == quote_ch {
                quote = None;
            }
            index += ch.len_utf8();
            continue;
        }
        match ch {
            '\'' | '"' => quote = Some(ch),
            '(' => paren_depth += 1,
            ')' => paren_depth = paren_depth.saturating_sub(1),
            '[' => bracket_depth += 1,
            ']' => bracket_depth = bracket_depth.saturating_sub(1),
            '{' => brace_depth += 1,
            '}' => brace_depth = brace_depth.saturating_sub(1),
            _ if ch == delimiter && paren_depth == 0 && bracket_depth == 0 && brace_depth == 0 => {
                parts.push(text[start..index].trim());
                start = index + ch.len_utf8();
            }
            _ => {}
        }
        index += ch.len_utf8();
    }
    parts.push(text[start..].trim());
    parts
}

fn parse_unique_key_literal(value: &str) -> Option<String> {
    if let Some(value) = quoted_string(value) {
        return Some(value);
    }
    let inner = value
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .or_else(|| {
            value
                .strip_prefix('(')
                .and_then(|value| value.strip_suffix(')'))
        })?;
    let parts = split_top_level_commas(inner)
        .into_iter()
        .map(str::trim)
        .map(quoted_string)
        .collect::<Option<Vec<_>>>()?;
    (!parts.is_empty()).then(|| parts.join(","))
}

fn quoted_string(value: &str) -> Option<String> {
    let mut chars = value.chars();
    let quote = chars.next()?;
    if quote != '\'' && quote != '"' {
        return None;
    }
    if !value.ends_with(quote) || value.len() < quote.len_utf8() * 2 {
        return None;
    }
    let inner = &value[quote.len_utf8()..value.len() - quote.len_utf8()];
    Some(inner.to_string())
}

fn ref_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r#"ref\s*\(\s*['"]([^'"]+)['"]\s*(?:,\s*['"]([^'"]+)['"]\s*)?\)"#)
            .expect("valid regex")
    })
}

fn source_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r#"source\s*\(\s*['"]([^'"]+)['"]\s*,\s*['"]([^'"]+)['"]\s*\)"#)
            .expect("valid regex")
    })
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

pub fn apply_dbt_project_configs(root: &Path, project: &mut DbtProject) -> Vec<MetadataWarning> {
    apply_dbt_project_configs_in_roots(root, &[root.to_path_buf()], project)
}

pub fn apply_dbt_project_configs_in_roots(
    root: &Path,
    scan_roots: &[PathBuf],
    project: &mut DbtProject,
) -> Vec<MetadataWarning> {
    let (project_files, warnings) =
        discover_dbt_project_files_in_roots_with_warnings(root, scan_roots);
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
    warnings
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
    fn parse_yaml_project_emits_warning_on_invalid_yaml() {
        let (project, warnings) =
            parse_yaml_project_with_warnings("{{not yaml", Path::new("models/schema.yml"));
        assert!(project.models.is_empty());
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].kind, MetadataWarningKind::YamlParseFailed);
    }

    #[test]
    fn extracts_dbt_sql_features() {
        let text = "{{ config(materialized='incremental', unique_key=\"id\") }} select * from {{ ref('stg_orders') }} join {{ ref('pkg', 'dim_orders') }} using (id) where {% if is_incremental() %} updated_at > current_date {% endif %}";
        let features = extract_sql_features(text);
        assert_eq!(features.refs[0].name, "stg_orders");
        assert_eq!(features.refs[1].package.as_deref(), Some("pkg"));
        assert_eq!(features.refs[1].name, "dim_orders");
        assert_eq!(features.config.materialized.as_deref(), Some("incremental"));
        assert_eq!(features.config.unique_key.as_deref(), Some("id"));
        assert!(features.uses_is_incremental);
    }

    #[test]
    fn extracts_inline_config_array_unique_key() {
        let text = "{{ config(materialized='incremental', unique_key=['account_id', \"event_date\"], incremental_strategy='merge') }} select 1";
        let features = extract_sql_features(text);
        assert_eq!(features.config.materialized.as_deref(), Some("incremental"));
        assert_eq!(
            features.config.unique_key.as_deref(),
            Some("account_id,event_date")
        );
        assert_eq!(
            features.config.incremental_strategy.as_deref(),
            Some("merge")
        );
    }

    #[test]
    fn extracts_multiline_inline_config_with_nested_calls() {
        let text = r#"{{
    config(
        materialized='incremental',
        unique_key=['transfer_id'],
        incremental_predicates=[incremental_predicate('DBT_INTERNAL_DEST.evt_block_time')]
    )
}}
select 1"#;
        let features = extract_sql_features(text);
        assert_eq!(features.config.materialized.as_deref(), Some("incremental"));
        assert_eq!(features.config.unique_key.as_deref(), Some("transfer_id"));
    }

    #[test]
    fn ignores_dynamic_inline_config_values_without_panic() {
        let text =
            "{{ config(materialized=var('materialization'), unique_key=var('key')) }} select 1";
        let features = extract_sql_features(text);
        assert_eq!(features.config.materialized, None);
        assert_eq!(features.config.unique_key, None);
    }

    #[test]
    fn later_inline_config_values_override_earlier_values() {
        let text = "{{ config(materialized='view', unique_key='old_id') }} {{ config(materialized='incremental', unique_key=['new_id']) }} select 1";
        let features = extract_sql_features(text);
        assert_eq!(features.config.materialized.as_deref(), Some("incremental"));
        assert_eq!(features.config.unique_key.as_deref(), Some("new_id"));
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
        let model = project.model_by_name("fct_sessions").unwrap();
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
        let model = project.model_by_name("fct_sessions").unwrap();
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
        let model = target.model_by_name("fct_sessions").unwrap();
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
        let model = target.model_by_name("fct_sessions").unwrap();
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
      "package_name": "pkg",
      "fqn": ["pkg", "marts", "fct_block_time"],
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
        let compiled = project.model_by_name("fct_block_time").unwrap();
        assert_eq!(
            compiled.compiled_code.as_deref(),
            Some("select tx_hash, block_time from dex.trades")
        );
        assert_eq!(compiled.identity(), "model.pkg.fct_block_time");
        assert_eq!(compiled.package_name.as_deref(), Some("pkg"));
        assert_eq!(compiled.fqn, vec!["pkg", "marts", "fct_block_time"]);
        let legacy = project.model_by_name("legacy_model").unwrap();
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

    #[test]
    fn top_level_assignment_requires_exactly_one_delimiter() {
        assert_eq!(split_top_level_assignment("materialized"), None);
        assert_eq!(split_top_level_assignment("a=b=c"), None);
        assert_eq!(
            split_top_level_assignment("materialized='incremental'"),
            Some(("materialized", "'incremental'"))
        );
    }
}
