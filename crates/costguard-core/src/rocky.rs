use anyhow::{Context, Result};
use costguard_project::{Framework, Materialization, ModelMetadata, ProjectGraph, Target};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::config::{DEFAULT_MAX_MANIFEST_BYTES, DEFAULT_MAX_TOTAL_BASE_BYTES};

pub const ROCKY_ARTIFACT_SCHEMA_VERSION: u8 = 1;

#[derive(Debug, Clone)]
pub struct RockyCaptureOptions {
    pub root: PathBuf,
    pub compile_path: PathBuf,
    pub rocky_config: PathBuf,
    pub models_dir: PathBuf,
    pub extra_inputs: Vec<PathBuf>,
    pub max_artifact_bytes: u64,
}

impl RockyCaptureOptions {
    pub fn new(root: PathBuf, compile_path: PathBuf) -> Self {
        Self {
            root,
            compile_path,
            rocky_config: PathBuf::from("rocky.toml"),
            models_dir: PathBuf::from("models"),
            extra_inputs: Vec::new(),
            max_artifact_bytes: DEFAULT_MAX_MANIFEST_BYTES,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RockyArtifactEnvelope {
    pub schema_version: u8,
    pub framework: String,
    pub costguard_version: String,
    pub rocky_version: String,
    pub git_commit: String,
    pub project_root: String,
    pub rocky_config: PathBuf,
    pub models_dir: PathBuf,
    pub inputs: Vec<RockyArtifactInput>,
    pub source_map: BTreeMap<String, PathBuf>,
    pub compile: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RockyArtifactInput {
    pub path: PathBuf,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Clone)]
struct RockyCompileModel {
    pub name: String,
    pub strategy: String,
    pub target: Target,
    pub depends_on: Vec<String>,
    pub tags: BTreeMap<String, String>,
    pub expanded_sql: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RockyIntegrityState {
    Verified,
    Missing,
    Invalid,
    Stale,
    NotConfigured,
}

#[derive(Debug, Clone, Serialize)]
pub struct RockyIntegrity {
    pub head: RockyIntegrityState,
    pub base: RockyIntegrityState,
    pub comparison_complete: bool,
    pub mismatch_count: usize,
    pub unclassified_head_findings: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub messages: Vec<String>,
    #[serde(skip)]
    pub(crate) unclassified_finding_ids: BTreeSet<String>,
}

impl Default for RockyIntegrity {
    fn default() -> Self {
        Self {
            head: RockyIntegrityState::NotConfigured,
            base: RockyIntegrityState::NotConfigured,
            comparison_complete: false,
            mismatch_count: 0,
            unclassified_head_findings: 0,
            messages: Vec::new(),
            unclassified_finding_ids: BTreeSet::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct RockyProject {
    pub graph: ProjectGraph,
    pub compiled_by_path: BTreeMap<PathBuf, String>,
    pub source_by_name: BTreeMap<String, PathBuf>,
    pub sealed_inputs: BTreeSet<PathBuf>,
    pub unresolved_dependencies: Vec<String>,
}

pub fn capture_rocky_artifact(options: &RockyCaptureOptions) -> Result<RockyArtifactEnvelope> {
    let root = options
        .root
        .canonicalize()
        .with_context(|| format!("failed to resolve project root {}", options.root.display()))?;
    let compile_path = resolve_under_root(&root, &options.compile_path, false)?;
    let observed = std::fs::metadata(&compile_path)
        .with_context(|| format!("failed to inspect {}", compile_path.display()))?
        .len();
    let max_artifact_bytes = effective_limit(options.max_artifact_bytes);
    if observed > max_artifact_bytes {
        anyhow::bail!(
            "Rocky compile output {} is {} bytes, exceeding limit of {} bytes",
            compile_path.display(),
            observed,
            max_artifact_bytes
        );
    }
    let compile_text = std::fs::read_to_string(&compile_path)
        .with_context(|| format!("failed to read {}", compile_path.display()))?;
    let compile: Value = serde_json::from_str(&compile_text)
        .with_context(|| format!("invalid Rocky compile JSON {}", compile_path.display()))?;
    let models = parse_compile_models(&compile)?;

    let config_path = resolve_under_root(&root, &options.rocky_config, true)?;
    if !config_path.is_file() {
        anyhow::bail!("Rocky config is not a file: {}", config_path.display());
    }
    let models_dir = resolve_under_root(&root, &options.models_dir, true)?;
    if !models_dir.is_dir() {
        anyhow::bail!(
            "Rocky models directory is not a directory: {}",
            models_dir.display()
        );
    }
    let source_map = resolve_model_sources(&root, &models_dir, &models)?;

    let mut input_paths = BTreeSet::new();
    collect_input(&root, &config_path, &mut input_paths)?;
    collect_input(&root, &models_dir, &mut input_paths)?;
    let compile_root = models_dir.parent().unwrap_or(&root);
    for default_dir in ["macros", "contracts", "groups"] {
        let path = compile_root.join(default_dir);
        if path.exists() {
            collect_input(&root, &path, &mut input_paths)?;
        }
    }
    for extra in &options.extra_inputs {
        let path = resolve_under_root(&root, extra, true)?;
        collect_input(&root, &path, &mut input_paths)?;
    }

    let max_file_bytes = costguard_scanner::effective_max_file_bytes(None);
    let git_commit = crate::git::head_commit(&root)?;
    let requests = input_paths
        .iter()
        .cloned()
        .map(|path| (path, max_file_bytes))
        .collect::<Vec<_>>();
    let head_contents = crate::git::files_at_commit_bytes_with_budget(
        &root,
        &git_commit,
        &requests,
        0,
        DEFAULT_MAX_TOTAL_BASE_BYTES,
    )?;
    let mut total = 0u64;
    let mut inputs = Vec::with_capacity(input_paths.len());
    for relative in input_paths {
        let absolute = root.join(&relative);
        let bytes = std::fs::read(&absolute)
            .with_context(|| format!("failed to read Rocky input {}", absolute.display()))?;
        let size = u64::try_from(bytes.len()).context("Rocky input is too large")?;
        if size > max_file_bytes {
            anyhow::bail!(
                "Rocky input {} is {} bytes, exceeding limit of {} bytes",
                relative.display(),
                size,
                max_file_bytes
            );
        }
        total = total
            .checked_add(size)
            .context("Rocky input size overflow")?;
        if total > DEFAULT_MAX_TOTAL_BASE_BYTES {
            anyhow::bail!(
                "Rocky inputs total {} bytes, exceeding limit of {} bytes",
                total,
                DEFAULT_MAX_TOTAL_BASE_BYTES
            );
        }
        let committed = head_contents
            .get(&relative)
            .and_then(|content| content.as_ref())
            .with_context(|| {
                format!("Rocky input {} is not tracked at HEAD", relative.display())
            })?;
        if committed != &bytes {
            anyhow::bail!(
                "Rocky input {} differs from committed HEAD",
                relative.display()
            );
        }
        inputs.push(RockyArtifactInput {
            path: relative,
            bytes: size,
            sha256: sha256(&bytes),
        });
    }

    let rocky_version = compile
        .get("version")
        .map(json_scalar)
        .unwrap_or_else(|| "unknown".into());
    let rocky_config = relative_to_root(&root, &config_path)?;
    let models_dir = relative_to_root(&root, &models_dir)?;
    let artifact = RockyArtifactEnvelope {
        schema_version: ROCKY_ARTIFACT_SCHEMA_VERSION,
        framework: "rocky".into(),
        costguard_version: env!("CARGO_PKG_VERSION").into(),
        rocky_version,
        git_commit,
        project_root: ".".into(),
        rocky_config,
        models_dir,
        inputs,
        source_map,
        compile,
    };
    let artifact_bytes = serde_json::to_vec_pretty(&artifact)?
        .len()
        .checked_add(1)
        .context("Rocky artifact size overflow")?;
    if u64::try_from(artifact_bytes).context("Rocky artifact is too large")? > max_artifact_bytes {
        anyhow::bail!(
            "sealed Rocky artifact is {} bytes, exceeding limit of {} bytes",
            artifact_bytes,
            max_artifact_bytes
        );
    }
    Ok(artifact)
}

pub fn write_rocky_artifact(path: &Path, artifact: &RockyArtifactEnvelope) -> Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent)
        .with_context(|| format!("failed to create {}", parent.display()))?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)
        .with_context(|| format!("failed to create temporary output in {}", parent.display()))?;
    serde_json::to_writer_pretty(temp.as_file_mut(), artifact)?;
    temp.write_all(b"\n")?;
    temp.as_file_mut().sync_all()?;
    temp.persist(path)
        .map(|_| ())
        .map_err(|error| error.error)
        .with_context(|| format!("failed to persist {}", path.display()))
}

pub fn load_rocky_artifact(path: &Path, max_bytes: u64) -> Result<RockyArtifactEnvelope> {
    let observed = std::fs::metadata(path)
        .with_context(|| format!("failed to inspect Rocky artifact {}", path.display()))?
        .len();
    let limit = effective_limit(max_bytes);
    if observed > limit {
        anyhow::bail!(
            "Rocky artifact {} is {} bytes, exceeding limit of {} bytes",
            path.display(),
            observed,
            limit
        );
    }
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read Rocky artifact {}", path.display()))?;
    let artifact: RockyArtifactEnvelope = serde_json::from_str(&text)
        .with_context(|| format!("invalid Rocky artifact {}", path.display()))?;
    validate_envelope(&artifact)?;
    Ok(artifact)
}

pub(crate) fn verify_rocky_artifact_current(
    root: &Path,
    artifact: &RockyArtifactEnvelope,
    max_total_bytes: u64,
) -> Result<RockyProject> {
    validate_envelope(artifact)?;
    let root = root
        .canonicalize()
        .with_context(|| format!("failed to resolve project root {}", root.display()))?;
    let head = crate::git::head_commit(&root)?;
    if artifact.git_commit != head {
        anyhow::bail!(
            "Rocky artifact commit {} does not match HEAD {}",
            artifact.git_commit,
            head
        );
    }
    let requests = artifact
        .inputs
        .iter()
        .map(|input| {
            (
                input.path.clone(),
                costguard_scanner::effective_max_file_bytes(None),
            )
        })
        .collect::<Vec<_>>();
    let committed =
        crate::git::files_at_commit_bytes_with_budget(&root, &head, &requests, 0, max_total_bytes)?;
    let mut total = 0u64;
    let max_file_bytes = costguard_scanner::effective_max_file_bytes(None);
    for input in &artifact.inputs {
        let absolute = safe_artifact_path(&root, &input.path)?;
        let metadata = std::fs::symlink_metadata(&absolute)
            .with_context(|| format!("missing sealed Rocky input {}", input.path.display()))?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            anyhow::bail!(
                "sealed Rocky input is not a regular file: {}",
                input.path.display()
            );
        }
        if metadata.len() > max_file_bytes {
            anyhow::bail!(
                "sealed Rocky input {} is {} bytes, exceeding limit of {} bytes",
                input.path.display(),
                metadata.len(),
                max_file_bytes
            );
        }
        let bytes = std::fs::read(&absolute)?;
        verify_input(input, &bytes)?;
        let committed_bytes = committed
            .get(&input.path)
            .and_then(|content| content.as_ref())
            .with_context(|| {
                format!(
                    "sealed Rocky input {} is missing from committed HEAD",
                    input.path.display()
                )
            })?;
        verify_input(input, committed_bytes).with_context(|| {
            format!(
                "sealed Rocky input {} does not match committed HEAD",
                input.path.display()
            )
        })?;
        total = total
            .checked_add(input.bytes)
            .context("Rocky artifact input size overflow")?;
        if total > max_total_bytes {
            anyhow::bail!(
                "Rocky artifact inputs total {} bytes, exceeding limit of {} bytes",
                total,
                max_total_bytes
            );
        }
    }
    project_from_envelope(artifact)
}

pub(crate) fn verify_rocky_artifact_at_commit(
    root: &Path,
    commit: &str,
    artifact: &RockyArtifactEnvelope,
    max_total_bytes: u64,
) -> Result<RockyProject> {
    validate_envelope(artifact)?;
    if artifact.git_commit != commit {
        anyhow::bail!(
            "base Rocky artifact commit {} does not match comparison commit {}",
            artifact.git_commit,
            commit
        );
    }
    let requests = artifact
        .inputs
        .iter()
        .map(|input| {
            (
                input.path.clone(),
                costguard_scanner::effective_max_file_bytes(None),
            )
        })
        .collect::<Vec<_>>();
    let contents =
        crate::git::files_at_commit_bytes_with_budget(root, commit, &requests, 0, max_total_bytes)?;
    for input in &artifact.inputs {
        let content = contents
            .get(&input.path)
            .and_then(|content| content.as_ref())
            .with_context(|| {
                format!(
                    "sealed Rocky input {} is missing at base commit",
                    input.path.display()
                )
            })?;
        verify_input(input, content)?;
    }
    project_from_envelope(artifact)
}

fn validate_envelope(artifact: &RockyArtifactEnvelope) -> Result<()> {
    if artifact.schema_version != ROCKY_ARTIFACT_SCHEMA_VERSION {
        anyhow::bail!(
            "unsupported Rocky artifact schema {}",
            artifact.schema_version
        );
    }
    if artifact.framework != "rocky" {
        anyhow::bail!(
            "Rocky artifact framework must be 'rocky', found '{}'",
            artifact.framework
        );
    }
    if artifact.git_commit.trim().is_empty() {
        anyhow::bail!("Rocky artifact git_commit is empty");
    }
    if artifact.inputs.is_empty() {
        anyhow::bail!("Rocky artifact has no sealed inputs");
    }
    validate_relative_path(&artifact.rocky_config)?;
    validate_relative_path(&artifact.models_dir)?;
    let mut paths = BTreeSet::new();
    let max_file_bytes = costguard_scanner::effective_max_file_bytes(None);
    for input in &artifact.inputs {
        validate_relative_path(&input.path)?;
        if !paths.insert(input.path.clone()) {
            anyhow::bail!("duplicate Rocky artifact input {}", input.path.display());
        }
        if input.sha256.len() != 64 || !input.sha256.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            anyhow::bail!(
                "Rocky artifact input {} has an invalid sha256",
                input.path.display()
            );
        }
        if input.bytes > max_file_bytes {
            anyhow::bail!(
                "Rocky artifact input {} exceeds the per-file limit of {} bytes",
                input.path.display(),
                max_file_bytes
            );
        }
    }
    if !paths.contains(&artifact.rocky_config) {
        anyhow::bail!("Rocky artifact does not seal its config file");
    }
    let models = parse_compile_models(&artifact.compile)?;
    let model_names = models
        .iter()
        .map(|model| model.name.as_str())
        .collect::<BTreeSet<_>>();
    let mapped_names = artifact
        .source_map
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    if model_names != mapped_names {
        anyhow::bail!("Rocky artifact source map does not exactly match compiled models");
    }
    let mut source_paths = BTreeSet::new();
    for path in artifact.source_map.values() {
        validate_relative_path(path)?;
        if !path.starts_with(&artifact.models_dir) {
            anyhow::bail!(
                "Rocky source {} is outside models directory {}",
                path.display(),
                artifact.models_dir.display()
            );
        }
        if !paths.contains(path) {
            anyhow::bail!("Rocky source {} is not a sealed input", path.display());
        }
        if !source_paths.insert(path) {
            anyhow::bail!("multiple Rocky models map to {}", path.display());
        }
    }
    Ok(())
}

fn project_from_envelope(artifact: &RockyArtifactEnvelope) -> Result<RockyProject> {
    let models = parse_compile_models(&artifact.compile)?;
    let names = models
        .iter()
        .map(|model| model.name.clone())
        .collect::<BTreeSet<_>>();
    let mut graph = ProjectGraph::default();
    let mut compiled_by_path = BTreeMap::new();
    let mut unresolved_dependencies = Vec::new();
    for model in models {
        let path = artifact
            .source_map
            .get(&model.name)
            .with_context(|| format!("Rocky model '{}' has no source mapping", model.name))?
            .clone();
        validate_relative_path(&path)?;
        let mut owners = Vec::new();
        let mut tags = Vec::new();
        for (key, value) in &model.tags {
            if key.eq_ignore_ascii_case("owner") {
                if !value.trim().is_empty() {
                    owners.push(value.clone());
                }
            } else {
                tags.push(format!("{key}={value}"));
            }
        }
        tags.sort();
        let depends_on = model
            .depends_on
            .iter()
            .map(|dependency| {
                if !names.contains(dependency) {
                    unresolved_dependencies.push(format!(
                        "Rocky model '{}' depends on unresolved model '{}'",
                        model.name, dependency
                    ));
                }
                format!("rocky.{dependency}")
            })
            .collect();
        let id = format!("rocky.{}", model.name);
        graph.insert_model(ModelMetadata {
            id,
            framework: Framework::Rocky,
            name: model.name.clone(),
            path: path.clone(),
            materialization: rocky_materialization(&model.strategy),
            strategy: Some(model.strategy),
            target: model.target,
            tags,
            owners,
            group: None,
            depends_on,
        });
        compiled_by_path.insert(path, model.expanded_sql);
    }
    graph.rebuild_indexes();
    Ok(RockyProject {
        graph,
        compiled_by_path,
        source_by_name: artifact.source_map.clone(),
        sealed_inputs: artifact
            .inputs
            .iter()
            .map(|input| input.path.clone())
            .collect(),
        unresolved_dependencies,
    })
}

fn parse_compile_models(compile: &Value) -> Result<Vec<RockyCompileModel>> {
    if compile.get("command").and_then(Value::as_str) != Some("compile") {
        anyhow::bail!("Rocky JSON is not compile output");
    }
    if compile
        .get("has_errors")
        .and_then(Value::as_bool)
        .unwrap_or(true)
    {
        anyhow::bail!("Rocky compile output contains errors");
    }
    let details = compile
        .get("models_detail")
        .and_then(Value::as_array)
        .context("Rocky compile output has no models_detail array")?;
    if details.is_empty() {
        anyhow::bail!("Rocky compile output contains no models");
    }
    let expanded = compile
        .get("expanded_sql")
        .and_then(Value::as_object)
        .context("Rocky compile output has no expanded_sql object; pass --expand-macros")?;
    let mut names = BTreeSet::new();
    let mut models = Vec::with_capacity(details.len());
    for detail in details {
        let name = detail
            .get("name")
            .and_then(Value::as_str)
            .context("Rocky model detail has no name")?
            .to_string();
        if !names.insert(name.clone()) {
            anyhow::bail!("duplicate Rocky model name '{name}'");
        }
        let expanded_sql = expanded
            .get(&name)
            .and_then(Value::as_str)
            .with_context(|| format!("Rocky model '{name}' has no expanded SQL"))?
            .to_string();
        if expanded_sql.trim().is_empty() {
            anyhow::bail!("Rocky model '{name}' has empty expanded SQL");
        }
        let strategy = detail
            .get("strategy")
            .map(strategy_name)
            .unwrap_or_else(|| "unknown".into());
        let target = detail.get("target").map(parse_target).unwrap_or_default();
        let depends_on = match detail.get("depends_on") {
            None => Vec::new(),
            Some(Value::Array(values)) => values
                .iter()
                .map(|value| {
                    value.as_str().map(str::to_string).with_context(|| {
                        format!("Rocky model '{name}' has a non-string dependency")
                    })
                })
                .collect::<Result<Vec<_>>>()?,
            Some(_) => anyhow::bail!("Rocky model '{name}' has invalid depends_on metadata"),
        };
        let tags = detail
            .get("tags")
            .and_then(Value::as_object)
            .map(|tags| {
                tags.iter()
                    .map(|(key, value)| (key.clone(), json_scalar(value)))
                    .collect()
            })
            .unwrap_or_default();
        models.push(RockyCompileModel {
            name,
            strategy,
            target,
            depends_on,
            tags,
            expanded_sql,
        });
    }
    if expanded.len() < models.len() {
        anyhow::bail!("Rocky expanded_sql does not cover every compiled model");
    }
    Ok(models)
}

fn parse_target(value: &Value) -> Target {
    let object = value.as_object();
    Target {
        catalog: object
            .and_then(|value| value.get("catalog"))
            .map(json_scalar)
            .filter(|value| !value.is_empty()),
        schema: object
            .and_then(|value| value.get("schema"))
            .map(json_scalar)
            .filter(|value| !value.is_empty()),
        object: object
            .and_then(|value| value.get("table").or_else(|| value.get("object")))
            .map(json_scalar)
            .filter(|value| !value.is_empty()),
    }
}

fn strategy_name(value: &Value) -> String {
    if let Some(value) = value.as_str() {
        return normalize_strategy(value);
    }
    let Some(object) = value.as_object() else {
        return "unknown".into();
    };
    if let Some(value) = object.get("type").and_then(Value::as_str) {
        return normalize_strategy(value);
    }
    object
        .keys()
        .next()
        .map(|value| normalize_strategy(value))
        .unwrap_or_else(|| "unknown".into())
}

fn normalize_strategy(value: &str) -> String {
    value.trim().replace(['-', ' '], "_").to_ascii_lowercase()
}

fn rocky_materialization(strategy: &str) -> Materialization {
    match strategy {
        "full_refresh" | "table" => Materialization::Table,
        "incremental" | "merge" | "time_interval" | "delete_insert" | "microbatch" => {
            Materialization::Incremental
        }
        "view" => Materialization::View,
        "materialized_view" => Materialization::MaterializedView,
        "ephemeral" => Materialization::Ephemeral,
        "dynamic_table" => Materialization::DynamicTable,
        "content_addressed" => Materialization::ContentAddressed,
        other => Materialization::Other(other.into()),
    }
}

fn resolve_model_sources(
    root: &Path,
    models_dir: &Path,
    models: &[RockyCompileModel],
) -> Result<BTreeMap<String, PathBuf>> {
    let mut files = BTreeSet::new();
    collect_model_files(models_dir, &mut files)?;
    let mut candidates = BTreeMap::<String, Vec<PathBuf>>::new();
    for path in files {
        let name = source_model_name(&path)?;
        candidates.entry(name).or_default().push(path);
    }
    for (name, paths) in &candidates {
        if paths.len() != 1 {
            anyhow::bail!(
                "Rocky model '{}' maps to multiple sources: {}",
                name,
                paths
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
    }
    let compiled_names = models
        .iter()
        .map(|model| model.name.as_str())
        .collect::<BTreeSet<_>>();
    let source_names = candidates
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    if compiled_names != source_names {
        let missing = compiled_names
            .difference(&source_names)
            .copied()
            .collect::<Vec<_>>();
        let unmapped = source_names
            .difference(&compiled_names)
            .copied()
            .collect::<Vec<_>>();
        anyhow::bail!(
            "Rocky source mapping is incomplete (missing sources: {}; unmapped sources: {})",
            missing.join(", "),
            unmapped.join(", ")
        );
    }
    let mut result = BTreeMap::new();
    for model in models {
        let paths = candidates
            .get(&model.name)
            .with_context(|| format!("no Rocky source maps to model '{}'", model.name))?;
        result.insert(model.name.clone(), relative_to_root(root, &paths[0])?);
    }
    Ok(result)
}

fn source_model_name(path: &Path) -> Result<String> {
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .with_context(|| format!("Rocky model path has no UTF-8 stem: {}", path.display()))?;
    let sidecar = path.with_extension("toml");
    let metadata = if sidecar.is_file() {
        Some(
            std::fs::read_to_string(&sidecar)
                .with_context(|| format!("failed to read {}", sidecar.display()))?,
        )
    } else {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        inline_frontmatter(&text).map(str::to_string)
    };
    if let Some(metadata) = metadata {
        let value: toml::Value = toml::from_str(&metadata)
            .with_context(|| format!("invalid Rocky metadata for {}", path.display()))?;
        if let Some(name) = value.get("name").and_then(toml::Value::as_str) {
            return Ok(name.to_string());
        }
    }
    Ok(stem.to_string())
}

fn inline_frontmatter(text: &str) -> Option<&str> {
    let after_start = text.trim_start().strip_prefix("---toml")?;
    let (frontmatter, _) = after_start.split_once("\n---")?;
    Some(frontmatter.trim())
}

fn collect_model_files(dir: &Path, files: &mut BTreeSet<PathBuf>) -> Result<()> {
    let mut entries = std::fs::read_dir(dir)
        .with_context(|| format!("failed to read {}", dir.display()))?
        .collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(|entry| entry.path());
    for entry in entries {
        let path = entry.path();
        let metadata = std::fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() {
            anyhow::bail!(
                "Rocky source symlinks are not supported: {}",
                path.display()
            );
        }
        if metadata.is_dir() {
            collect_model_files(&path, files)?;
        } else if matches!(
            path.extension().and_then(|extension| extension.to_str()),
            Some("sql" | "rocky")
        ) {
            files.insert(path);
        }
    }
    Ok(())
}

fn collect_input(root: &Path, path: &Path, files: &mut BTreeSet<PathBuf>) -> Result<()> {
    let metadata = std::fs::symlink_metadata(path)
        .with_context(|| format!("failed to inspect Rocky input {}", path.display()))?;
    if metadata.file_type().is_symlink() {
        anyhow::bail!("Rocky input symlinks are not supported: {}", path.display());
    }
    if metadata.is_file() {
        files.insert(relative_to_root(root, path)?);
        return Ok(());
    }
    if !metadata.is_dir() {
        anyhow::bail!(
            "Rocky input is not a regular file or directory: {}",
            path.display()
        );
    }
    let mut entries = std::fs::read_dir(path)
        .with_context(|| format!("failed to read Rocky input {}", path.display()))?
        .collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(|entry| entry.path());
    for entry in entries {
        collect_input(root, &entry.path(), files)?;
    }
    Ok(())
}

fn resolve_under_root(root: &Path, path: &Path, must_exist: bool) -> Result<PathBuf> {
    if path
        .components()
        .any(|component| component == std::path::Component::ParentDir)
    {
        anyhow::bail!("path traversal is not allowed: {}", path.display());
    }
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };
    if !must_exist && !joined.exists() {
        anyhow::bail!("path does not exist: {}", joined.display());
    }
    reject_symlink_components(root, &joined)?;
    let canonical = joined
        .canonicalize()
        .with_context(|| format!("failed to resolve {}", joined.display()))?;
    if !canonical.starts_with(root) {
        anyhow::bail!("path escapes project root: {}", path.display());
    }
    Ok(canonical)
}

fn relative_to_root(root: &Path, path: &Path) -> Result<PathBuf> {
    path.strip_prefix(root)
        .map(Path::to_path_buf)
        .with_context(|| format!("path escapes project root: {}", path.display()))
}

fn safe_artifact_path(root: &Path, path: &Path) -> Result<PathBuf> {
    validate_relative_path(path)?;
    let absolute = root.join(path);
    reject_symlink_components(root, &absolute)?;
    let canonical = absolute
        .canonicalize()
        .with_context(|| format!("failed to resolve sealed Rocky input {}", path.display()))?;
    if !canonical.starts_with(root) {
        anyhow::bail!(
            "sealed Rocky input escapes project root: {}",
            path.display()
        );
    }
    Ok(canonical)
}

fn reject_symlink_components(root: &Path, path: &Path) -> Result<()> {
    let relative = path
        .strip_prefix(root)
        .with_context(|| format!("path escapes project root: {}", path.display()))?;
    let mut current = root.to_path_buf();
    for component in relative.components() {
        if !matches!(
            component,
            std::path::Component::Normal(_) | std::path::Component::CurDir
        ) {
            anyhow::bail!("path traversal is not allowed: {}", path.display());
        }
        current.push(component);
        if current.exists()
            && std::fs::symlink_metadata(&current)?
                .file_type()
                .is_symlink()
        {
            anyhow::bail!(
                "symlinked Rocky paths are not supported: {}",
                current.display()
            );
        }
    }
    Ok(())
}

fn validate_relative_path(path: &Path) -> Result<()> {
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir
                    | std::path::Component::RootDir
                    | std::path::Component::Prefix(_)
            )
        })
    {
        anyhow::bail!(
            "Rocky artifact path is not project-relative: {}",
            path.display()
        );
    }
    Ok(())
}

fn verify_input(input: &RockyArtifactInput, bytes: &[u8]) -> Result<()> {
    if u64::try_from(bytes.len()).ok() != Some(input.bytes) || sha256(bytes) != input.sha256 {
        anyhow::bail!(
            "sealed Rocky input {} does not match artifact hash",
            input.path.display()
        );
    }
    Ok(())
}

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn json_scalar(value: &Value) -> String {
    value
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| value.to_string())
}

fn effective_limit(value: u64) -> u64 {
    if value == 0 {
        DEFAULT_MAX_MANIFEST_BYTES
    } else {
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn compile_json(models: Value, expanded_sql: Value) -> Value {
        serde_json::json!({
            "version": "0.9.0",
            "command": "compile",
            "has_errors": false,
            "models_detail": models,
            "expanded_sql": expanded_sql,
            "future_field": {"is": "ignored"}
        })
    }

    fn model_detail(name: &str, strategy: Value) -> Value {
        serde_json::json!({
            "name": name,
            "strategy": strategy,
            "target": {"catalog": "lake", "schema": "mart", "table": name},
            "depends_on": [],
            "tags": {}
        })
    }

    fn fixture_repo() -> tempfile::TempDir {
        let repo = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(repo.path().join("models")).unwrap();
        std::fs::create_dir_all(repo.path().join("target")).unwrap();
        std::fs::write(repo.path().join("rocky.toml"), "version = 1\n").unwrap();
        std::fs::write(
            repo.path().join("models/orders.rocky"),
            "from raw_orders\nselect *\n",
        )
        .unwrap();
        git(repo.path(), &["init"]);
        git(repo.path(), &["config", "user.email", "test@example.com"]);
        git(repo.path(), &["config", "user.name", "Test"]);
        git(repo.path(), &["add", "rocky.toml", "models/orders.rocky"]);
        git(repo.path(), &["commit", "-m", "fixture"]);
        write_compile(repo.path(), "select * from raw_orders");
        repo
    }

    fn write_compile(root: &Path, sql: &str) {
        std::fs::write(
            root.join("target/compile.json"),
            serde_json::to_vec(&compile_json(
                serde_json::json!([{
                    "name": "orders",
                    "strategy": {"type": "view"},
                    "target": {"catalog": "lake", "schema": "mart", "table": "orders"},
                    "depends_on": [],
                    "tags": {"owner": "@finance", "layer": "gold"}
                }]),
                serde_json::json!({"orders": sql}),
            ))
            .unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn parses_current_and_future_strategy_shapes() {
        assert_eq!(strategy_name(&serde_json::json!("merge")), "merge");
        assert_eq!(
            strategy_name(&serde_json::json!({"type": "time_interval"})),
            "time_interval"
        );
        assert_eq!(
            strategy_name(&serde_json::json!({"FutureMode": {"x": 1}})),
            "futuremode"
        );
    }

    #[test]
    fn parses_inline_frontmatter_name() {
        let text = "---toml\nname = \"renamed\"\n---\nselect 1";
        assert_eq!(inline_frontmatter(text), Some("name = \"renamed\""));
    }

    #[test]
    fn rejects_parent_paths() {
        assert!(validate_relative_path(Path::new("../secret")).is_err());
    }

    #[test]
    fn captures_and_verifies_committed_rocky_project() {
        let repo = fixture_repo();
        let artifact = capture_rocky_artifact(&RockyCaptureOptions::new(
            repo.path().to_path_buf(),
            PathBuf::from("target/compile.json"),
        ))
        .unwrap();
        assert_eq!(
            artifact.source_map["orders"],
            PathBuf::from("models/orders.rocky")
        );
        let project = verify_rocky_artifact_current(repo.path(), &artifact, 1024 * 1024).unwrap();
        let model = project.graph.model("rocky.orders").unwrap();
        assert_eq!(model.framework, Framework::Rocky);
        assert_eq!(model.owners, vec!["@finance"]);
        assert_eq!(model.tags, vec!["layer=gold"]);
        assert!(verify_rocky_artifact_current(repo.path(), &artifact, 1).is_err());

        std::fs::write(repo.path().join("models/orders.rocky"), "from changed\n").unwrap();
        assert!(verify_rocky_artifact_current(repo.path(), &artifact, 1024 * 1024).is_err());
    }

    #[test]
    fn current_verification_also_requires_committed_head_hashes() {
        let repo = fixture_repo();
        let mut artifact = capture_rocky_artifact(&RockyCaptureOptions::new(
            repo.path().to_path_buf(),
            PathBuf::from("target/compile.json"),
        ))
        .unwrap();
        let changed = b"from changed\nselect *\n";
        std::fs::write(repo.path().join("models/orders.rocky"), changed).unwrap();
        let input = artifact
            .inputs
            .iter_mut()
            .find(|input| input.path == Path::new("models/orders.rocky"))
            .unwrap();
        input.bytes = changed.len() as u64;
        input.sha256 = sha256(changed);

        let error = verify_rocky_artifact_current(repo.path(), &artifact, 1024 * 1024).unwrap_err();
        assert!(format!("{error:#}").contains("committed HEAD"), "{error:#}");
    }

    #[test]
    fn sealed_binary_inputs_verify_against_git() {
        let repo = fixture_repo();
        std::fs::write(repo.path().join("seed.bin"), [0, 159, 146, 150]).unwrap();
        git(repo.path(), &["add", "seed.bin"]);
        git(repo.path(), &["commit", "-m", "binary input"]);
        let mut options = RockyCaptureOptions::new(
            repo.path().to_path_buf(),
            PathBuf::from("target/compile.json"),
        );
        options.extra_inputs.push(PathBuf::from("seed.bin"));
        let artifact = capture_rocky_artifact(&options).unwrap();

        verify_rocky_artifact_current(repo.path(), &artifact, 1024 * 1024).unwrap();
    }

    #[test]
    fn compile_payload_validation_rejects_incomplete_or_failed_output() {
        let valid = compile_json(
            serde_json::json!([model_detail(
                "orders",
                serde_json::json!({"type": "future_strategy", "option": true})
            )]),
            serde_json::json!({"orders": "select 1"}),
        );
        let parsed = parse_compile_models(&valid).unwrap();
        assert_eq!(parsed[0].strategy, "future_strategy");

        for invalid in [
            serde_json::json!({"command": "run", "has_errors": false}),
            serde_json::json!({
                "command": "compile",
                "has_errors": true,
                "models_detail": [model_detail("orders", serde_json::json!("view"))],
                "expanded_sql": {"orders": "select 1"}
            }),
            compile_json(
                serde_json::json!([model_detail("orders", serde_json::json!("view"))]),
                serde_json::json!({}),
            ),
            compile_json(
                serde_json::json!([model_detail("orders", serde_json::json!("view"))]),
                serde_json::json!({"orders": "  "}),
            ),
            compile_json(
                serde_json::json!([{
                    "name": "orders",
                    "strategy": "view",
                    "depends_on": [7]
                }]),
                serde_json::json!({"orders": "select 1"}),
            ),
        ] {
            assert!(parse_compile_models(&invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn source_mapping_supports_sidecars_and_rejects_ambiguity() {
        let root = tempfile::tempdir().unwrap();
        let models_dir = root.path().join("models");
        std::fs::create_dir(&models_dir).unwrap();
        std::fs::write(models_dir.join("physical.sql"), "select 1").unwrap();
        std::fs::write(models_dir.join("physical.toml"), "name = \"logical\"\n").unwrap();
        let models = vec![RockyCompileModel {
            name: "logical".into(),
            strategy: "view".into(),
            target: Target::default(),
            depends_on: Vec::new(),
            tags: BTreeMap::new(),
            expanded_sql: "select 1".into(),
        }];
        let mapped = resolve_model_sources(root.path(), &models_dir, &models).unwrap();
        assert_eq!(mapped["logical"], PathBuf::from("models/physical.sql"));

        std::fs::write(
            models_dir.join("other.rocky"),
            "---toml\nname = \"logical\"\n---\nfrom source",
        )
        .unwrap();
        assert!(resolve_model_sources(root.path(), &models_dir, &models).is_err());
        std::fs::remove_file(models_dir.join("other.rocky")).unwrap();
        std::fs::write(models_dir.join("unused.sql"), "select 2").unwrap();
        assert!(resolve_model_sources(root.path(), &models_dir, &models).is_err());
        std::fs::remove_file(models_dir.join("unused.sql")).unwrap();
        let mut missing = models;
        missing[0].name = "missing".into();
        assert!(resolve_model_sources(root.path(), &models_dir, &missing).is_err());
    }

    #[test]
    fn capture_rejects_untracked_dirty_traversing_and_oversized_inputs() {
        let repo = fixture_repo();
        std::fs::write(repo.path().join("defaults.toml"), "x = 1\n").unwrap();
        let mut options = RockyCaptureOptions::new(
            repo.path().to_path_buf(),
            PathBuf::from("target/compile.json"),
        );
        options.extra_inputs = vec![PathBuf::from("defaults.toml")];
        assert!(capture_rocky_artifact(&options)
            .unwrap_err()
            .to_string()
            .contains("not tracked"));

        git(repo.path(), &["add", "defaults.toml"]);
        git(repo.path(), &["commit", "-m", "defaults"]);
        std::fs::write(repo.path().join("defaults.toml"), "x = 2\n").unwrap();
        assert!(capture_rocky_artifact(&options)
            .unwrap_err()
            .to_string()
            .contains("differs from committed HEAD"));

        std::fs::write(repo.path().join("defaults.toml"), "x = 1\n").unwrap();
        options.extra_inputs = vec![PathBuf::from("models/../rocky.toml")];
        assert!(capture_rocky_artifact(&options)
            .unwrap_err()
            .to_string()
            .contains("traversal"));

        options.extra_inputs.clear();
        options.max_artifact_bytes = 1;
        assert!(capture_rocky_artifact(&options)
            .unwrap_err()
            .to_string()
            .contains("exceeding limit"));
    }

    #[cfg(unix)]
    #[test]
    fn capture_and_verification_reject_symlinked_paths() {
        use std::os::unix::fs::symlink;

        let repo = fixture_repo();
        symlink(repo.path().join("models"), repo.path().join("macros")).unwrap();
        let options = RockyCaptureOptions::new(
            repo.path().to_path_buf(),
            PathBuf::from("target/compile.json"),
        );
        assert!(capture_rocky_artifact(&options)
            .unwrap_err()
            .to_string()
            .contains("symlink"));

        std::fs::remove_file(repo.path().join("macros")).unwrap();
        let artifact = capture_rocky_artifact(&options).unwrap();
        std::fs::rename(repo.path().join("models"), repo.path().join("real_models")).unwrap();
        symlink(repo.path().join("real_models"), repo.path().join("models")).unwrap();
        assert!(
            verify_rocky_artifact_current(repo.path(), &artifact, 1024 * 1024)
                .unwrap_err()
                .to_string()
                .contains("symlink")
        );
    }

    #[test]
    fn artifact_is_deterministic_size_bounded_and_atomically_replaceable() {
        let repo = fixture_repo();
        let compile_bytes = std::fs::metadata(repo.path().join("target/compile.json"))
            .unwrap()
            .len();
        let mut options = RockyCaptureOptions::new(
            repo.path().to_path_buf(),
            PathBuf::from("target/compile.json"),
        );
        let first = capture_rocky_artifact(&options).unwrap();
        let second = capture_rocky_artifact(&options).unwrap();
        assert_eq!(
            serde_json::to_vec(&first).unwrap(),
            serde_json::to_vec(&second).unwrap()
        );

        let output = repo.path().join("target/costguard-rocky.json");
        std::fs::write(&output, "old").unwrap();
        write_rocky_artifact(&output, &first).unwrap();
        assert_eq!(
            load_rocky_artifact(&output, DEFAULT_MAX_MANIFEST_BYTES)
                .unwrap()
                .git_commit,
            first.git_commit
        );

        options.max_artifact_bytes = compile_bytes + 1;
        assert!(capture_rocky_artifact(&options)
            .unwrap_err()
            .to_string()
            .contains("sealed Rocky artifact"));
    }

    #[test]
    fn envelope_validation_rejects_partial_or_unsealed_source_maps() {
        let repo = fixture_repo();
        let mut artifact = capture_rocky_artifact(&RockyCaptureOptions::new(
            repo.path().to_path_buf(),
            PathBuf::from("target/compile.json"),
        ))
        .unwrap();
        artifact.source_map.clear();
        assert!(validate_envelope(&artifact).is_err());

        artifact
            .source_map
            .insert("orders".into(), PathBuf::from("models/not-sealed.rocky"));
        assert!(validate_envelope(&artifact).is_err());
    }

    #[test]
    fn base_verification_uses_immutable_git_inputs() {
        let repo = fixture_repo();
        let artifact = capture_rocky_artifact(&RockyCaptureOptions::new(
            repo.path().to_path_buf(),
            PathBuf::from("target/compile.json"),
        ))
        .unwrap();
        let base = artifact.git_commit.clone();
        std::fs::write(repo.path().join("models/orders.rocky"), "from changed\n").unwrap();
        git(repo.path(), &["add", "models/orders.rocky"]);
        git(repo.path(), &["commit", "-m", "head"]);

        assert!(verify_rocky_artifact_current(repo.path(), &artifact, 1024 * 1024).is_err());
        assert!(
            verify_rocky_artifact_at_commit(repo.path(), &base, &artifact, 1024 * 1024).is_ok()
        );
        assert!(
            verify_rocky_artifact_at_commit(repo.path(), "HEAD", &artifact, 1024 * 1024).is_err()
        );
    }

    #[test]
    fn materialization_normalization_preserves_unknown_strategies() {
        for (strategy, expected) in [
            ("full_refresh", Materialization::Table),
            ("incremental", Materialization::Incremental),
            ("merge", Materialization::Incremental),
            ("time_interval", Materialization::Incremental),
            ("delete_insert", Materialization::Incremental),
            ("microbatch", Materialization::Incremental),
            ("view", Materialization::View),
            ("materialized_view", Materialization::MaterializedView),
            ("dynamic_table", Materialization::DynamicTable),
            ("content_addressed", Materialization::ContentAddressed),
            ("ephemeral", Materialization::Ephemeral),
            ("future", Materialization::Other("future".into())),
        ] {
            assert_eq!(rocky_materialization(strategy), expected);
        }
    }

    fn git(root: &Path, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(root)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
