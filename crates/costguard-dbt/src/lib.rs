//! dbt project metadata parsing.
//!
//! Loads manifest JSON, YAML schema files, and `dbt_project.yml` folder
//! configs to build a [`DbtProject`] graph and extract Jinja-level SQL
//! features (refs, sources, incremental blocks).

mod graph;
mod manifest;
mod project_config;
mod sql_features;
mod yaml;

pub use graph::{build_downstream_counts, build_downstream_ids, DependencyGraph};
pub use manifest::{parse_manifest, parse_manifest_text, parse_manifest_with_limit};
use project_config::{
    apply_folder_config_to_model, discover_dbt_project_files_in_roots_with_warnings,
    find_dbt_project_for_model, resolve_folder_config_for_model,
};
pub use project_config::{
    parse_config_node as parse_dbt_config, parse_dbt_project_with_warnings, DbtProjectFile,
    FolderConfigMap,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
pub use sql_features::{extract_incremental_block, extract_sql_features};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
pub use yaml::{
    compiled_code_by_model_path, merge_yaml_project, parse_yaml_project,
    parse_yaml_project_with_warnings, synthesized_model_id,
};

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
    pub partition_by: Option<String>,
    pub cluster_by: Option<String>,
    pub full_refresh: Option<bool>,
    pub on_schema_change: Option<String>,
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
    pub macros: HashMap<String, DbtMacro>,
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
    pub depends_on_macros: HashMap<String, Vec<String>>,
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
    pub partition_by: Option<String>,
    pub cluster_by: Option<String>,
    pub full_refresh: Option<bool>,
    pub on_schema_change: Option<String>,
    pub compiled_code: Option<String>,
    pub tags: Vec<String>,
    pub owners: Vec<String>,
    pub group: Option<String>,
    pub columns: Vec<DbtColumn>,
    pub tests: Vec<DbtTest>,
    pub refs: Vec<String>,
    pub sources: Vec<DbtSourceRef>,
    pub checksum: Option<String>,
    pub checksum_kind: Option<String>,
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
pub struct DbtMacro {
    pub unique_id: String,
    pub name: String,
    pub path: Option<PathBuf>,
    pub depends_on_macros: Vec<String>,
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
    pub owners: Vec<String>,
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
    StaleManifest,
    ManifestChecksumMismatch,
    FileSkipped,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetadataWarning {
    pub kind: MetadataWarningKind,
    pub path: Option<PathBuf>,
    pub message: String,
}

pub fn apply_dbt_project_configs_in_roots(
    root: &Path,
    scan_roots: &[PathBuf],
    project: &mut DbtProject,
) -> Vec<MetadataWarning> {
    let (project_files, warnings) =
        discover_dbt_project_files_in_roots_with_warnings(root, scan_roots);
    apply_dbt_project_configs_from_files(root, &project_files, project);
    warnings
}

pub fn apply_dbt_project_configs_from_files(
    root: &Path,
    project_files: &[DbtProjectFile],
    project: &mut DbtProject,
) {
    for model in project.models.values_mut() {
        let Some(model_path) = model.path.as_ref() else {
            continue;
        };
        let absolute_model_path = root.join(model_path);
        let Some(project_file) = find_dbt_project_for_model(&absolute_model_path, project_files)
        else {
            continue;
        };
        let folder_config = resolve_folder_config_for_model(&absolute_model_path, project_file);
        apply_folder_config_to_model(model, &folder_config);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sql_features::split_top_level_assignment;

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
    group: finance
    meta:
      owners: ["@finance", "data@example.com"]
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
    owner:
      name: Analytics
      email: analytics@example.com
    depends_on:
      - ref('fct_sessions')
"#;
        let project = parse_yaml_project(yaml);
        let model = project.model_by_name("fct_sessions").unwrap();
        assert_eq!(model.tags, vec!["mart", "critical"]);
        assert_eq!(model.owners, vec!["@finance", "data@example.com"]);
        assert_eq!(model.group.as_deref(), Some("finance"));
        assert_eq!(model.columns[0].tests[0].name, "not_null");
        assert_eq!(project.sources["raw"].tables, vec!["events"]);
        assert_eq!(
            project.exposures["dashboard"].depends_on[0],
            "ref('fct_sessions')"
        );
        assert_eq!(
            project.exposures["dashboard"].owners,
            vec!["Analytics", "analytics@example.com"]
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
      "meta": { "owner": "@manifest-owner" },
      "group": "finance",
      "config": { "materialized": "incremental", "meta": { "owners": ["data@example.com"] } },
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
        assert_eq!(compiled.owners, vec!["@manifest-owner", "data@example.com"]);
        assert_eq!(compiled.group.as_deref(), Some("finance"));
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
    fn parses_manifest_checksum_macros_and_macro_dependencies() {
        let manifest = r#"{
  "nodes": {
    "model.pkg.fct_orders": {
      "resource_type": "model",
      "name": "fct_orders",
      "original_file_path": "models/fct_orders.sql",
      "checksum": { "name": "sha256", "checksum": "abc123" },
      "depends_on": {
        "nodes": ["source.pkg.raw.events"],
        "macros": ["macro.pkg.generate_schema_name"]
      }
    }
  },
  "macros": {
    "macro.pkg.generate_schema_name": {
      "name": "generate_schema_name",
      "original_file_path": "macros/generate_schema_name.sql",
      "depends_on": { "macros": ["macro.dbt.generate_schema_name"] }
    }
  }
}"#;
        let project = parse_manifest_text(manifest).expect("parse manifest");
        let model = project.model_by_name("fct_orders").unwrap();
        assert_eq!(model.checksum.as_deref(), Some("abc123"));
        assert_eq!(model.checksum_kind.as_deref(), Some("sha256"));
        assert_eq!(
            project.graph.depends_on_macros["model.pkg.fct_orders"],
            vec!["macro.pkg.generate_schema_name"]
        );
        let dbt_macro = project
            .macros
            .get("macro.pkg.generate_schema_name")
            .unwrap();
        assert_eq!(dbt_macro.name, "generate_schema_name");
        assert_eq!(
            dbt_macro.path.as_deref(),
            Some(Path::new("macros/generate_schema_name.sql"))
        );
        assert_eq!(
            dbt_macro.depends_on_macros,
            vec!["macro.dbt.generate_schema_name"]
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
