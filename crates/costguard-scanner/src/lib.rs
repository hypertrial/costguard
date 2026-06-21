//! Project file discovery and classification.
//!
//! Walks the project tree (or reads explicit paths), classifies files as SQL,
//! dbt YAML, Python, or manifest JSON, and returns [`ProjectFile`] records
//! ready for downstream parsing.

use anyhow::{Context, Result};
use costguard_diagnostics::LineIndex;
use ignore::{DirEntry, WalkBuilder};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use serde_yaml::Value;
use std::path::{Path, PathBuf};

pub const DEFAULT_MAX_FILE_BYTES: u64 = 5 * 1024 * 1024;
const DEFAULT_EXCLUDED_DIRS: &[&str] = &[
    ".git",
    "target",
    "dbt_packages",
    "node_modules",
    "dist",
    "build",
    ".venv",
    "__pycache__",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileKind {
    Sql,
    DbtSqlModel,
    DbtYaml,
    Python,
    ManifestJson,
    Other,
}

#[derive(Debug, Clone)]
pub struct ProjectFile {
    pub path: PathBuf,
    pub root_relative_path: PathBuf,
    pub kind: FileKind,
    pub text: String,
    pub line_index: LineIndex,
}

#[derive(Debug, Clone)]
pub struct SkippedFile {
    pub path: PathBuf,
    pub root_relative_path: PathBuf,
    pub reason: String,
}

#[derive(Debug, Clone, Default)]
pub struct DiscoveryOptions {
    pub ignore: Vec<PathBuf>,
    pub max_file_bytes: u64,
}

impl DiscoveryOptions {
    pub fn from_scan(ignore: Vec<PathBuf>, max_file_bytes: Option<u64>) -> Self {
        Self {
            ignore,
            max_file_bytes: max_file_bytes.unwrap_or(0),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct DiscoveryResult {
    pub files: Vec<ProjectFile>,
    pub skipped_files: Vec<SkippedFile>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ScanCounts {
    pub sql: usize,
    pub yaml: usize,
    pub python: usize,
    pub manifest: usize,
    pub other: usize,
}

impl ScanCounts {
    pub fn from_files(files: &[ProjectFile]) -> Self {
        let mut counts = Self::default();
        for file in files {
            match file.kind {
                FileKind::Sql | FileKind::DbtSqlModel => counts.sql += 1,
                FileKind::DbtYaml => counts.yaml += 1,
                FileKind::Python => counts.python += 1,
                FileKind::ManifestJson => counts.manifest += 1,
                FileKind::Other => counts.other += 1,
            }
        }
        counts
    }
}

pub fn discover_with_options(
    root: &Path,
    requested_paths: &[PathBuf],
    options: &DiscoveryOptions,
) -> Result<DiscoveryResult> {
    let root = root
        .canonicalize()
        .with_context(|| format!("failed to resolve project root {}", root.display()))?;
    let requested_scan_roots = if requested_paths.is_empty() {
        vec![root.to_path_buf()]
    } else {
        requested_paths
            .iter()
            .map(|path| {
                if path.is_absolute() {
                    path.clone()
                } else {
                    root.join(path)
                }
            })
            .collect()
    };

    let (ignored, max_file_bytes) = resolve_discovery_options(&root, options);
    let mut paths = Vec::new();
    let mut skipped_files = Vec::new();
    for requested_scan_root in requested_scan_roots {
        let scan_root = canonical_within_root(&root, &requested_scan_root)
            .with_context(|| format!("invalid scan path {}", requested_scan_root.display()))?;
        let mut builder = WalkBuilder::new(&scan_root);
        builder.standard_filters(true).hidden(false);
        let ignored_for_filter = ignored.clone();
        for entry in builder
            .filter_entry(move |entry| should_descend(entry, &ignored_for_filter))
            .build()
        {
            let entry = entry?;
            if !entry.file_type().is_some_and(|kind| kind.is_file()) {
                continue;
            }
            let requested_path = entry.into_path();
            let path = match canonical_within_root(&root, &requested_path) {
                Ok(path) => path,
                Err(err) => {
                    skipped_files.push(SkippedFile {
                        root_relative_path: requested_path
                            .strip_prefix(&root)
                            .unwrap_or(&requested_path)
                            .to_path_buf(),
                        path: requested_path,
                        reason: err.to_string(),
                    });
                    continue;
                }
            };
            collect_scannable_path(
                &root,
                path,
                &ignored,
                max_file_bytes,
                &mut paths,
                &mut skipped_files,
            );
        }
    }
    Ok(finish_discovery(&root, paths, skipped_files))
}

pub fn read_existing_paths_with_options(
    root: &Path,
    requested_paths: &[PathBuf],
    options: &DiscoveryOptions,
) -> Result<DiscoveryResult> {
    let root = root
        .canonicalize()
        .with_context(|| format!("failed to resolve project root {}", root.display()))?;
    let (ignored, max_file_bytes) = resolve_discovery_options(&root, options);
    let mut skipped_files = Vec::new();
    let mut paths = Vec::new();
    for requested in requested_paths {
        let requested_path = if requested.is_absolute() {
            requested.clone()
        } else {
            root.join(requested)
        };
        if !requested_path.exists() {
            continue;
        }
        let path = match canonical_within_root(&root, &requested_path) {
            Ok(path) => path,
            Err(err) => {
                skipped_files.push(SkippedFile {
                    root_relative_path: requested.clone(),
                    path: requested_path,
                    reason: err.to_string(),
                });
                continue;
            }
        };
        if !path.is_file() {
            continue;
        }
        collect_scannable_path(
            &root,
            path,
            &ignored,
            max_file_bytes,
            &mut paths,
            &mut skipped_files,
        );
    }
    Ok(finish_discovery(&root, paths, skipped_files))
}

fn resolve_discovery_options(root: &Path, options: &DiscoveryOptions) -> (Vec<PathBuf>, u64) {
    let ignored = normalize_ignored(root, &options.ignore);
    let max_file_bytes = effective_max_file_bytes(Some(options.max_file_bytes));
    (ignored, max_file_bytes)
}

pub fn effective_max_file_bytes(configured: Option<u64>) -> u64 {
    configured
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_MAX_FILE_BYTES)
}

fn collect_scannable_path(
    root: &Path,
    path: PathBuf,
    ignored: &[PathBuf],
    max_file_bytes: u64,
    paths: &mut Vec<(PathBuf, FileKind)>,
    skipped_files: &mut Vec<SkippedFile>,
) {
    if is_ignored_path(&path, ignored) {
        return;
    }
    let kind = classify(&path, root);
    if kind == FileKind::Other {
        return;
    }
    if file_too_large(&path, max_file_bytes) {
        skipped_files.push(skipped_file(root, path, max_file_bytes));
        return;
    }
    paths.push((path, kind));
}

fn finish_discovery(
    root: &Path,
    mut paths: Vec<(PathBuf, FileKind)>,
    mut skipped_files: Vec<SkippedFile>,
) -> DiscoveryResult {
    paths.sort_by(|left, right| left.0.cmp(&right.0));
    paths.dedup_by(|left, right| left.0 == right.0);
    let loaded = load_project_files(root, paths);
    skipped_files.extend(loaded.skipped_files);
    DiscoveryResult {
        files: loaded.files,
        skipped_files,
    }
}

fn load_project_files(root: &Path, paths: Vec<(PathBuf, FileKind)>) -> DiscoveryResult {
    let loaded = paths
        .into_par_iter()
        .map(|(path, kind)| {
            let text = match std::fs::read_to_string(&path) {
                Ok(text) => text,
                Err(err) => {
                    return Err(SkippedFile {
                        root_relative_path: path.strip_prefix(root).unwrap_or(&path).to_path_buf(),
                        path: path.clone(),
                        reason: format!("failed to read {}: {err}", path.display()),
                    });
                }
            };
            let kind = if kind == FileKind::DbtYaml && !is_dbt_yaml(&path, root, &text) {
                FileKind::Other
            } else {
                kind
            };
            if kind == FileKind::Other {
                return Ok(None);
            }
            let root_relative_path = path.strip_prefix(root).unwrap_or(&path).to_path_buf();
            let line_index = LineIndex::new(&text);
            Ok(Some(ProjectFile {
                path,
                root_relative_path,
                kind,
                text,
                line_index,
            }))
        })
        .collect::<Vec<Result<Option<ProjectFile>, SkippedFile>>>();
    let mut result = DiscoveryResult::default();
    for item in loaded {
        match item {
            Ok(Some(file)) => result.files.push(file),
            Ok(None) => {}
            Err(skipped) => result.skipped_files.push(skipped),
        }
    }
    result
        .files
        .sort_by(|left, right| left.path.cmp(&right.path));
    result
}

fn canonical_within_root(root: &Path, path: &Path) -> Result<PathBuf> {
    let canonical_root = root
        .canonicalize()
        .with_context(|| format!("failed to resolve project root {}", root.display()))?;
    let canonical = path
        .canonicalize()
        .with_context(|| format!("failed to resolve {}", path.display()))?;
    if !canonical.starts_with(&canonical_root) {
        anyhow::bail!(
            "path {} resolves outside project root {}",
            path.display(),
            canonical_root.display()
        );
    }
    Ok(canonical)
}

pub fn classify(path: &Path, root: &Path) -> FileKind {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let relative = costguard_diagnostics::posix_path(path.strip_prefix(root).unwrap_or(path))
        .to_ascii_lowercase();

    if file_name == "manifest.json" {
        return FileKind::ManifestJson;
    }
    match extension.as_str() {
        "sql" if relative.contains("models/") && !relative.contains("/macros/") => {
            FileKind::DbtSqlModel
        }
        "sql" => FileKind::Sql,
        "yml" | "yaml" => FileKind::DbtYaml,
        "py" => FileKind::Python,
        _ => FileKind::Other,
    }
}

fn normalize_ignored(root: &Path, ignore: &[PathBuf]) -> Vec<PathBuf> {
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

fn should_descend(entry: &DirEntry, ignored: &[PathBuf]) -> bool {
    let path = entry.path();
    if is_ignored_path(path, ignored) {
        return false;
    }
    if entry.file_type().is_some_and(|kind| kind.is_dir()) {
        if let Some(name) = path.file_name().and_then(|name| name.to_str()) {
            return !DEFAULT_EXCLUDED_DIRS.contains(&name);
        }
    }
    true
}

fn is_ignored_path(path: &Path, ignored: &[PathBuf]) -> bool {
    ignored
        .iter()
        .any(|ignored_path| path.starts_with(ignored_path))
}

fn file_too_large(path: &Path, max_file_bytes: u64) -> bool {
    path.metadata()
        .map(|metadata| metadata.len() > max_file_bytes)
        .unwrap_or(false)
}

fn skipped_file(root: &Path, path: PathBuf, max_file_bytes: u64) -> SkippedFile {
    let root_relative_path = path.strip_prefix(root).unwrap_or(&path).to_path_buf();
    SkippedFile {
        path,
        root_relative_path,
        reason: format!("file exceeds max scan size of {max_file_bytes} bytes"),
    }
}

fn is_dbt_yaml(path: &Path, root: &Path, text: &str) -> bool {
    if path.file_name().and_then(|name| name.to_str()) == Some("dbt_project.yml") {
        return true;
    }
    let relative = costguard_diagnostics::posix_path(path.strip_prefix(root).unwrap_or(path))
        .to_ascii_lowercase();
    if relative.contains("/models/") || relative.starts_with("models/") {
        return true;
    }
    let Ok(value) = serde_yaml::from_str::<Value>(text) else {
        return true;
    };
    let Some(object) = value.as_mapping() else {
        return false;
    };
    [
        "models",
        "sources",
        "exposures",
        "seeds",
        "snapshots",
        "macros",
        "tests",
    ]
    .iter()
    .any(|key| object.contains_key(Value::String((*key).to_string())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn classifies_dbt_sql() {
        assert_eq!(
            classify(Path::new("/r/models/marts/fct.sql"), Path::new("/r")),
            FileKind::DbtSqlModel
        );
    }

    #[test]
    fn macro_models_sql_is_generic_sql() {
        assert_eq!(
            classify(
                Path::new("/r/dbt_subprojects/dex/macros/models/decoding/evm.sql"),
                Path::new("/r")
            ),
            FileKind::Sql
        );
    }

    #[test]
    fn discovery_ignores_configured_paths_before_loading() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let root = tempdir.path();
        fs::create_dir_all(root.join("models")).expect("models");
        fs::create_dir_all(root.join("ignored")).expect("ignored");
        fs::write(root.join("models/a.sql"), "select 1").expect("write model");
        fs::write(root.join("ignored/b.sql"), "select 2").expect("write ignored");

        let result = discover_with_options(
            root,
            &[],
            &DiscoveryOptions::from_scan(vec![PathBuf::from("ignored")], None),
        )
        .expect("discover");

        assert_eq!(result.files.len(), 1);
        assert_eq!(
            result.files[0].root_relative_path,
            PathBuf::from("models/a.sql")
        );
    }

    #[test]
    fn non_dbt_yaml_is_not_loaded_as_context() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let root = tempdir.path();
        fs::create_dir_all(root.join(".github/workflows")).expect("workflow dir");
        fs::write(
            root.join(".github/workflows/ci.yml"),
            "name: ci\non: [push]\n",
        )
        .expect("write ci");

        let result =
            discover_with_options(root, &[], &DiscoveryOptions::default()).expect("discover");
        assert!(result.files.is_empty(), "{result:?}");
    }

    #[test]
    fn large_files_are_reported_as_skipped() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let root = tempdir.path();
        fs::create_dir_all(root.join("models")).expect("models");
        fs::write(root.join("models/large.sql"), "select 1\nselect 2\n").expect("large");

        let result = discover_with_options(
            root,
            &[],
            &DiscoveryOptions {
                ignore: Vec::new(),
                max_file_bytes: 4,
            },
        )
        .expect("discover");

        assert!(result.files.is_empty());
        assert_eq!(result.skipped_files.len(), 1);
        assert_eq!(
            result.skipped_files[0].root_relative_path,
            PathBuf::from("models/large.sql")
        );
    }

    #[test]
    fn requested_path_outside_root_is_rejected() {
        let root = tempfile::tempdir().expect("root");
        let outside = tempfile::tempdir().expect("outside");
        fs::write(outside.path().join("model.sql"), "select 1").expect("write outside");
        let err = discover_with_options(
            root.path(),
            &[outside.path().join("model.sql")],
            &DiscoveryOptions::default(),
        )
        .expect_err("outside path must fail");
        assert!(
            format!("{err:#}").contains("outside project root"),
            "{err:#}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn symlink_target_outside_root_is_reported_as_skipped() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().expect("root");
        let outside = tempfile::tempdir().expect("outside");
        fs::create_dir_all(root.path().join("models")).expect("models");
        fs::write(outside.path().join("secret.sql"), "select 1").expect("write outside");
        symlink(
            outside.path().join("secret.sql"),
            root.path().join("models/linked.sql"),
        )
        .expect("symlink");

        let result = read_existing_paths_with_options(
            root.path(),
            &[PathBuf::from("models/linked.sql")],
            &DiscoveryOptions::default(),
        )
        .expect("discover");
        assert!(result.files.is_empty());
        assert_eq!(result.skipped_files.len(), 1);
        assert!(result.skipped_files[0]
            .reason
            .contains("outside project root"));
    }
}
