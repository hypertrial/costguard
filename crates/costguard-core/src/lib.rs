use anyhow::{Context, Result};
use costguard_dbt::{parse_manifest, DbtModel, DbtProject};
use costguard_diagnostics::{apply_suppressions, Diagnostic, Severity};
use costguard_rules::{RuleOverrides, RuleRegistry, Warehouse};
use costguard_scanner::{discover, FileKind, ProjectFile, ScanCounts};
use costguard_sql::{analyze_sql, SqlDialect, SqlDocument};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::str::FromStr;

pub use costguard_dbt::{
    DbtColumn, DbtConfig, DbtExposure, DbtGraph, DbtProject as DbtProjectModel, DbtRef, DbtSource,
    DbtSourceRef, DbtSqlFeatures, DbtTest,
};
pub use costguard_diagnostics::{Confidence, LineIndex, Span};
pub use costguard_rules::{Rule, RuleContext, RuleMetadata};
pub use costguard_scanner::{FileKind as ProjectFileKind, ProjectFile as ScannedProjectFile};
pub use costguard_sql::{CteFeature, ExpressionFeature, JoinFeature, SqlFeatures, WindowFeature};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OutputFormat {
    #[default]
    Text,
    Json,
}

impl FromStr for OutputFormat {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "text" => Ok(Self::Text),
            "json" => Ok(Self::Json),
            other => Err(format!("unknown output format '{other}'")),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ScanConfig {
    pub root: PathBuf,
    pub paths: Vec<PathBuf>,
    pub warehouse: Warehouse,
    pub dialect: SqlDialect,
    pub format: OutputFormat,
    pub manifest_path: Option<PathBuf>,
    pub ignore: Vec<PathBuf>,
    pub base_branch: Option<String>,
    pub changed_only: bool,
    pub fail_on: Option<Severity>,
    pub rule_overrides: RuleOverrides,
}

impl Default for ScanConfig {
    fn default() -> Self {
        Self {
            root: PathBuf::from("."),
            paths: Vec::new(),
            warehouse: Warehouse::Generic,
            dialect: SqlDialect::Generic,
            format: OutputFormat::Text,
            manifest_path: None,
            ignore: Vec::new(),
            base_branch: None,
            changed_only: false,
            fail_on: Some(Severity::High),
            rule_overrides: RuleOverrides::default(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Project {
    pub root: PathBuf,
    pub files: Vec<ProjectFile>,
    pub dbt: Option<DbtProject>,
}

#[derive(Debug, Clone)]
pub struct ScanResult {
    pub diagnostics: Vec<Diagnostic>,
    pub counts: ScanCounts,
    pub pr_summary: Option<PrSummary>,
}

impl ScanResult {
    pub fn should_fail(&self, fail_on: Option<Severity>) -> bool {
        let Some(threshold) = fail_on else {
            return false;
        };
        self.diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity >= threshold)
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct PrSummary {
    pub changed_files: Vec<PathBuf>,
    pub changed_models: Vec<String>,
    pub affected_downstream: Vec<String>,
    pub recommended_dbt_command: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct FileConfig {
    pub warehouse: Option<String>,
    pub dialect: Option<String>,
    pub scan: Option<ScanSection>,
    pub output: Option<OutputSection>,
    pub dbt: Option<DbtSection>,
    pub rules: Option<RuleOverrides>,
}

#[derive(Debug, Deserialize, Default)]
pub struct ScanSection {
    pub paths: Option<Vec<PathBuf>>,
    pub ignore: Option<Vec<PathBuf>>,
}

#[derive(Debug, Deserialize, Default)]
pub struct OutputSection {
    pub format: Option<String>,
    pub fail_on: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct DbtSection {
    pub manifest_path: Option<PathBuf>,
}

pub fn load_config(root: &Path) -> Result<FileConfig> {
    let path = root.join("costguard.toml");
    if !path.exists() {
        return Ok(FileConfig::default());
    }
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    toml::from_str(&text).with_context(|| format!("invalid config {}", path.display()))
}

pub fn apply_file_config(mut config: ScanConfig, file_config: FileConfig) -> Result<ScanConfig> {
    if let Some(warehouse) = file_config.warehouse {
        config.warehouse = warehouse.parse().map_err(anyhow::Error::msg)?;
    }
    if let Some(dialect) = file_config.dialect {
        config.dialect = dialect.parse().map_err(anyhow::Error::msg)?;
    }
    if let Some(scan) = file_config.scan {
        if let Some(paths) = scan.paths {
            config.paths = paths;
        }
        if let Some(ignore) = scan.ignore {
            config.ignore = ignore;
        }
    }
    if let Some(output) = file_config.output {
        if let Some(format) = output.format {
            config.format = format.parse().map_err(anyhow::Error::msg)?;
        }
        if let Some(fail_on) = output.fail_on {
            config.fail_on = Some(fail_on.parse().map_err(anyhow::Error::msg)?);
        }
    }
    if let Some(dbt) = file_config.dbt {
        config.manifest_path = dbt.manifest_path;
    }
    if let Some(rules) = file_config.rules {
        config.rule_overrides = rules
            .into_iter()
            .map(|(key, value)| (key.to_ascii_uppercase(), value))
            .collect();
    }
    Ok(config)
}

pub fn scan(config: &ScanConfig) -> Result<ScanResult> {
    let root = config
        .root
        .canonicalize()
        .with_context(|| format!("failed to resolve root {}", config.root.display()))?;
    let mut files = discover(&root, &config.paths)?;
    if !config.ignore.is_empty() {
        let ignored = config
            .ignore
            .iter()
            .map(|path| {
                if path.is_absolute() {
                    path.clone()
                } else {
                    root.join(path)
                }
            })
            .collect::<Vec<_>>();
        files.retain(|file| {
            !ignored
                .iter()
                .any(|ignored_path| file.path.starts_with(ignored_path))
        });
    }
    let pr_summary = if config.changed_only {
        let base = config.base_branch.as_deref().unwrap_or("main");
        let changed_files = costguard_git::changed_files(&root, base)?;
        let changed: HashSet<PathBuf> = changed_files.iter().cloned().collect();
        files.retain(|file| changed.contains(&file.root_relative_path));
        Some(PrSummary {
            changed_files,
            ..PrSummary::default()
        })
    } else {
        None
    };

    let counts = ScanCounts::from_files(&files);
    let dbt = load_dbt_project(&root, config, &files)?;
    let project = Project {
        root: root.clone(),
        files,
        dbt,
    };
    let sql_documents = analyze_sql_documents(&project, config.dialect);
    let sql_by_path = sql_documents
        .iter()
        .map(|doc| (doc.path.clone(), doc))
        .collect::<HashMap<_, _>>();
    let dbt_by_path = dbt_models_by_path(&project);
    let registry = RuleRegistry::default_rules();
    let mut diagnostics = Vec::new();

    for file in &project.files {
        let sql = sql_by_path.get(&file.path).copied();
        let dbt_model = dbt_by_path.get(&file.root_relative_path).copied();
        let ctx = costguard_rules::RuleContext {
            warehouse: config.warehouse,
            file,
            sql,
            dbt_model,
            all_sql: &sql_documents,
            overrides: &config.rule_overrides,
        };
        let file_diagnostics = apply_suppressions(&file.text, registry.run(&ctx));
        diagnostics.extend(file_diagnostics);
    }

    diagnostics.sort_by(|left, right| {
        left.path
            .cmp(&right.path)
            .then(left.line.cmp(&right.line))
            .then(left.rule_id.cmp(&right.rule_id))
    });

    let pr_summary = pr_summary.map(|summary| enrich_pr_summary(summary, &project));
    Ok(ScanResult {
        diagnostics,
        counts,
        pr_summary,
    })
}

pub fn explain(config: &ScanConfig, path: &Path) -> Result<ScanResult> {
    let mut config = config.clone();
    config.paths = vec![path.to_path_buf()];
    scan(&config)
}

pub fn rules() -> Vec<RuleMetadata> {
    RuleRegistry::default_rules().metadata()
}

fn load_dbt_project(
    root: &Path,
    config: &ScanConfig,
    files: &[ProjectFile],
) -> Result<Option<DbtProject>> {
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

    let mut project = match manifest_path {
        Some(path) => parse_manifest(&path)?,
        None => DbtProject::default(),
    };

    for file in files {
        if file.kind != FileKind::DbtSqlModel {
            continue;
        }
        let features = costguard_dbt::extract_sql_features(&file.text);
        let name = file
            .path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or_default()
            .to_string();
        let model = project
            .models
            .entry(name.clone())
            .or_insert_with(|| DbtModel {
                name,
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
        model.refs = features
            .refs
            .into_iter()
            .map(|reference| reference.name)
            .collect();
        model.sources = features.sources;
    }

    if project.models.is_empty() {
        Ok(None)
    } else {
        Ok(Some(project))
    }
}

fn analyze_sql_documents(project: &Project, dialect: SqlDialect) -> Vec<SqlDocument> {
    project
        .files
        .iter()
        .filter(|file| matches!(file.kind, FileKind::Sql | FileKind::DbtSqlModel))
        .map(|file| analyze_sql(file.path.clone(), &file.text, dialect, &file.line_index))
        .collect()
}

fn dbt_models_by_path(project: &Project) -> HashMap<PathBuf, &DbtModel> {
    project
        .dbt
        .as_ref()
        .into_iter()
        .flat_map(|dbt| dbt.models.values())
        .filter_map(|model| model.path.as_ref().map(|path| (path.clone(), model)))
        .collect()
}

fn enrich_pr_summary(mut summary: PrSummary, project: &Project) -> PrSummary {
    let Some(dbt) = &project.dbt else {
        return summary;
    };
    let changed_set: HashSet<PathBuf> = summary.changed_files.iter().cloned().collect();
    summary.changed_models = dbt
        .models
        .values()
        .filter(|model| {
            model
                .path
                .as_ref()
                .is_some_and(|path| changed_set.contains(path))
        })
        .map(|model| model.name.clone())
        .collect();
    summary.changed_models.sort();

    let changed_model_set: HashSet<&str> =
        summary.changed_models.iter().map(String::as_str).collect();
    summary.affected_downstream = dbt
        .models
        .values()
        .filter(|model| {
            model
                .refs
                .iter()
                .any(|reference| changed_model_set.contains(reference.as_str()))
        })
        .map(|model| model.name.clone())
        .collect();
    summary.affected_downstream.sort();

    if !summary.changed_models.is_empty() {
        let selectors = summary
            .changed_models
            .iter()
            .map(|model| format!("{model}+"))
            .collect::<Vec<_>>()
            .join(" ");
        summary.recommended_dbt_command = Some(format!("dbt build --select {selectors}"));
    }
    summary
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_format_parses() {
        assert_eq!("json".parse::<OutputFormat>().unwrap(), OutputFormat::Json);
    }
}
