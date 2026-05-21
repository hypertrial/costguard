use anyhow::{Context, Result};
use costguard_dbt::{
    extract_sql_features, merge_yaml_project, parse_manifest, parse_yaml_project, DbtModel,
    DbtProject,
};
use costguard_diagnostics::{apply_suppressions, Diagnostic, Severity};
use costguard_rules::{ProjectIndexes, RuleOverrides, RuleRegistry, Warehouse};
use costguard_scanner::{discover, read_existing_paths, FileKind, ProjectFile, ScanCounts};
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
    Github,
    Markdown,
}

impl FromStr for OutputFormat {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "text" => Ok(Self::Text),
            "json" => Ok(Self::Json),
            "github" => Ok(Self::Github),
            "markdown" | "md" => Ok(Self::Markdown),
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
    pub affected_exposures: Vec<String>,
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
    let ignored = ignored_paths(&root, &config.ignore);
    let (mut files, mut context_files, pr_summary) = if config.changed_only {
        let base = config.base_branch.as_deref().unwrap_or("main");
        let changed_files = costguard_git::changed_files(&root, base)?;
        let files = read_existing_paths(&root, &changed_files)?;
        let context_files = discover(&root, &config.paths)?
            .into_iter()
            .filter(is_dbt_context_file)
            .collect::<Vec<_>>();
        (
            files,
            context_files,
            Some(PrSummary {
                changed_files,
                ..PrSummary::default()
            }),
        )
    } else {
        let files = discover(&root, &config.paths)?;
        (files.clone(), files, None)
    };

    if !ignored.is_empty() {
        files.retain(|file| !is_ignored(file, &ignored));
        context_files.retain(|file| !is_ignored(file, &ignored));
    };

    let counts = ScanCounts::from_files(&files);
    let dbt = load_dbt_project(&root, config, &context_files)?;
    let project = Project {
        root: root.clone(),
        files,
        dbt,
    };
    let sql_documents = analyze_sql_documents(&project.files, config.dialect);
    let context_sql_documents = analyze_sql_documents(&context_files, config.dialect);
    let project_indexes = ProjectIndexes::from_sql_documents(&context_sql_documents);
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
            all_sql: &context_sql_documents,
            project_indexes: &project_indexes,
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

fn ignored_paths(root: &Path, ignore: &[PathBuf]) -> Vec<PathBuf> {
    ignore
        .iter()
        .map(|path| {
            if path.is_absolute() {
                path.clone()
            } else {
                root.join(path)
            }
        })
        .collect()
}

fn is_ignored(file: &ProjectFile, ignored: &[PathBuf]) -> bool {
    ignored
        .iter()
        .any(|ignored_path| file.path.starts_with(ignored_path))
}

fn is_dbt_context_file(file: &ProjectFile) -> bool {
    matches!(
        file.kind,
        FileKind::DbtSqlModel | FileKind::DbtYaml | FileKind::ManifestJson
    )
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
        if file.kind == FileKind::DbtYaml {
            merge_yaml_project(&mut project, parse_yaml_project(&file.text));
        }
    }

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
        if model.refs.is_empty() {
            model.refs = features
                .refs
                .into_iter()
                .map(|reference| reference.name)
                .collect();
        }
        if model.sources.is_empty() {
            model.sources = features.sources;
        }
        let dependency_key = model.node_id.clone().unwrap_or_else(|| model.name.clone());
        let dependencies = model.refs.clone();
        project
            .graph
            .depends_on
            .entry(dependency_key)
            .or_insert(dependencies);
    }

    if project.models.is_empty() {
        Ok(None)
    } else {
        Ok(Some(project))
    }
}

fn analyze_sql_documents(files: &[ProjectFile], dialect: SqlDialect) -> Vec<SqlDocument> {
    files
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

    let graph = DbtDependencyGraph::from_project(dbt);
    summary.affected_downstream = graph.transitive_downstream(&summary.changed_models);
    summary.affected_downstream.sort();
    summary.affected_exposures = graph.affected_exposures(
        &summary
            .changed_models
            .iter()
            .chain(summary.affected_downstream.iter())
            .cloned()
            .collect::<Vec<_>>(),
    );
    summary.affected_exposures.sort();

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

struct DbtDependencyGraph {
    reverse_dependencies: HashMap<String, Vec<String>>,
    exposure_dependencies: HashMap<String, Vec<String>>,
}

impl DbtDependencyGraph {
    fn from_project(project: &DbtProject) -> Self {
        let node_to_model_name = project
            .models
            .values()
            .filter_map(|model| {
                model
                    .node_id
                    .as_ref()
                    .map(|node_id| (node_id.clone(), model.name.clone()))
            })
            .collect::<HashMap<_, _>>();
        let mut reverse_dependencies: HashMap<String, Vec<String>> = HashMap::new();

        for model in project.models.values() {
            let dependent = model.name.clone();
            for reference in &model.refs {
                reverse_dependencies
                    .entry(reference.clone())
                    .or_default()
                    .push(dependent.clone());
            }
            let graph_key = model.node_id.as_ref().unwrap_or(&model.name);
            if let Some(depends_on) = project.graph.depends_on.get(graph_key) {
                for dependency in depends_on {
                    let dependency_name =
                        normalize_dependency_name(dependency, &node_to_model_name);
                    reverse_dependencies
                        .entry(dependency_name)
                        .or_default()
                        .push(dependent.clone());
                }
            }
        }

        for dependents in reverse_dependencies.values_mut() {
            dependents.sort();
            dependents.dedup();
        }

        let exposure_dependencies = project
            .exposures
            .values()
            .map(|exposure| {
                (
                    exposure.name.clone(),
                    exposure
                        .depends_on
                        .iter()
                        .map(|dependency| {
                            normalize_dependency_name(dependency, &node_to_model_name)
                        })
                        .collect::<Vec<_>>(),
                )
            })
            .collect();

        Self {
            reverse_dependencies,
            exposure_dependencies,
        }
    }

    fn transitive_downstream(&self, changed_models: &[String]) -> Vec<String> {
        let changed = changed_models.iter().cloned().collect::<HashSet<_>>();
        let mut seen = HashSet::new();
        let mut stack = changed_models.to_vec();
        while let Some(model) = stack.pop() {
            if let Some(dependents) = self.reverse_dependencies.get(&model) {
                for dependent in dependents {
                    if changed.contains(dependent) || !seen.insert(dependent.clone()) {
                        continue;
                    }
                    stack.push(dependent.clone());
                }
            }
        }
        let mut downstream = seen.into_iter().collect::<Vec<_>>();
        downstream.sort();
        downstream
    }

    fn affected_exposures(&self, affected_models: &[String]) -> Vec<String> {
        let affected = affected_models.iter().cloned().collect::<HashSet<_>>();
        let mut exposures = self
            .exposure_dependencies
            .iter()
            .filter(|(_, dependencies)| {
                dependencies
                    .iter()
                    .any(|dependency| affected.contains(dependency))
            })
            .map(|(name, _)| name.clone())
            .collect::<Vec<_>>();
        exposures.sort();
        exposures
    }
}

fn normalize_dependency_name(
    dependency: &str,
    node_to_model_name: &HashMap<String, String>,
) -> String {
    if let Some(name) = node_to_model_name.get(dependency) {
        return name.clone();
    }
    let trimmed = dependency.trim();
    if let Some(inner) = trimmed
        .strip_prefix("ref(")
        .and_then(|value| value.strip_suffix(')'))
    {
        return inner.trim_matches(['"', '\'']).to_string();
    }
    trimmed
        .rsplit('.')
        .next()
        .unwrap_or(trimmed)
        .trim_matches(['"', '\''])
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_format_parses() {
        assert_eq!("json".parse::<OutputFormat>().unwrap(), OutputFormat::Json);
        assert_eq!(
            "github".parse::<OutputFormat>().unwrap(),
            OutputFormat::Github
        );
        assert_eq!(
            "markdown".parse::<OutputFormat>().unwrap(),
            OutputFormat::Markdown
        );
    }

    #[test]
    fn pr_summary_uses_transitive_manifest_graph_and_exposures() {
        let mut dbt = DbtProject::default();
        dbt.models.insert(
            "a".into(),
            DbtModel {
                node_id: Some("model.pkg.a".into()),
                name: "a".into(),
                path: Some(PathBuf::from("models/a.sql")),
                ..DbtModel::default()
            },
        );
        dbt.models.insert(
            "b".into(),
            DbtModel {
                node_id: Some("model.pkg.b".into()),
                name: "b".into(),
                path: Some(PathBuf::from("models/b.sql")),
                ..DbtModel::default()
            },
        );
        dbt.models.insert(
            "c".into(),
            DbtModel {
                node_id: Some("model.pkg.c".into()),
                name: "c".into(),
                path: Some(PathBuf::from("models/c.sql")),
                ..DbtModel::default()
            },
        );
        dbt.graph
            .depends_on
            .insert("model.pkg.b".into(), vec!["model.pkg.a".into()]);
        dbt.graph
            .depends_on
            .insert("model.pkg.c".into(), vec!["model.pkg.b".into()]);
        dbt.exposures.insert(
            "growth_dashboard".into(),
            costguard_dbt::DbtExposure {
                name: "growth_dashboard".into(),
                depends_on: vec!["model.pkg.c".into()],
            },
        );
        let project = Project {
            root: PathBuf::from("."),
            files: Vec::new(),
            dbt: Some(dbt),
        };
        let summary = enrich_pr_summary(
            PrSummary {
                changed_files: vec![PathBuf::from("models/a.sql")],
                ..PrSummary::default()
            },
            &project,
        );
        assert_eq!(summary.changed_models, vec!["a"]);
        assert_eq!(summary.affected_downstream, vec!["b", "c"]);
        assert_eq!(summary.affected_exposures, vec!["growth_dashboard"]);
    }

    #[test]
    fn pr_summary_falls_back_to_sql_refs_without_manifest() {
        let mut dbt = DbtProject::default();
        dbt.models.insert(
            "a".into(),
            DbtModel {
                name: "a".into(),
                path: Some(PathBuf::from("models/a.sql")),
                ..DbtModel::default()
            },
        );
        dbt.models.insert(
            "b".into(),
            DbtModel {
                name: "b".into(),
                path: Some(PathBuf::from("models/b.sql")),
                refs: vec!["a".into()],
                ..DbtModel::default()
            },
        );
        dbt.models.insert(
            "c".into(),
            DbtModel {
                name: "c".into(),
                path: Some(PathBuf::from("models/c.sql")),
                refs: vec!["b".into()],
                ..DbtModel::default()
            },
        );
        let project = Project {
            root: PathBuf::from("."),
            files: Vec::new(),
            dbt: Some(dbt),
        };
        let summary = enrich_pr_summary(
            PrSummary {
                changed_files: vec![PathBuf::from("models/a.sql")],
                ..PrSummary::default()
            },
            &project,
        );
        assert_eq!(summary.affected_downstream, vec!["b", "c"]);
    }
}
