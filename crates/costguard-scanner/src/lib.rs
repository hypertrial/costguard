use anyhow::{Context, Result};
use costguard_diagnostics::LineIndex;
use ignore::{DirEntry, WalkBuilder};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use serde_yaml::Value;
use std::path::{Path, PathBuf};

const DEFAULT_MAX_FILE_BYTES: u64 = 5 * 1024 * 1024;
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
    pub fn with_ignore(ignore: Vec<PathBuf>) -> Self {
        Self {
            ignore,
            max_file_bytes: DEFAULT_MAX_FILE_BYTES,
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

pub fn discover(root: &Path, requested_paths: &[PathBuf]) -> Result<Vec<ProjectFile>> {
    Ok(discover_with_options(root, requested_paths, &DiscoveryOptions::default())?.files)
}

pub fn discover_with_options(
    root: &Path,
    requested_paths: &[PathBuf],
    options: &DiscoveryOptions,
) -> Result<DiscoveryResult> {
    let scan_roots = if requested_paths.is_empty() {
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

    let mut paths = Vec::new();
    let mut skipped_files = Vec::new();
    let ignored = normalize_ignored(root, &options.ignore);
    let max_file_bytes = if options.max_file_bytes == 0 {
        DEFAULT_MAX_FILE_BYTES
    } else {
        options.max_file_bytes
    };
    for scan_root in scan_roots {
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
            let path = entry.into_path();
            if is_ignored_path(&path, &ignored) {
                continue;
            }
            let kind = classify(&path, root);
            if kind != FileKind::Other {
                if file_too_large(&path, max_file_bytes) {
                    skipped_files.push(skipped_file(root, path, max_file_bytes));
                    continue;
                }
                paths.push((path, kind));
            }
        }
    }
    paths.sort_by(|left, right| left.0.cmp(&right.0));
    paths.dedup_by(|left, right| left.0 == right.0);
    Ok(DiscoveryResult {
        files: load_project_files(root, paths)?,
        skipped_files,
    })
}

pub fn read_existing_paths(root: &Path, requested_paths: &[PathBuf]) -> Result<Vec<ProjectFile>> {
    Ok(
        read_existing_paths_with_options(root, requested_paths, &DiscoveryOptions::default())?
            .files,
    )
}

pub fn read_existing_paths_with_options(
    root: &Path,
    requested_paths: &[PathBuf],
    options: &DiscoveryOptions,
) -> Result<DiscoveryResult> {
    let ignored = normalize_ignored(root, &options.ignore);
    let max_file_bytes = if options.max_file_bytes == 0 {
        DEFAULT_MAX_FILE_BYTES
    } else {
        options.max_file_bytes
    };
    let mut skipped_files = Vec::new();
    let mut paths = requested_paths
        .iter()
        .map(|path| {
            if path.is_absolute() {
                path.clone()
            } else {
                root.join(path)
            }
        })
        .filter(|path| path.is_file() && !is_ignored_path(path, &ignored))
        .filter_map(|path| {
            let kind = classify(&path, root);
            if kind != FileKind::Other && file_too_large(&path, max_file_bytes) {
                skipped_files.push(skipped_file(root, path, max_file_bytes));
                return None;
            }
            (kind != FileKind::Other).then_some((path, kind))
        })
        .collect::<Vec<_>>();
    paths.sort_by(|left, right| left.0.cmp(&right.0));
    paths.dedup_by(|left, right| left.0 == right.0);
    Ok(DiscoveryResult {
        files: load_project_files(root, paths)?,
        skipped_files,
    })
}

fn load_project_files(root: &Path, paths: Vec<(PathBuf, FileKind)>) -> Result<Vec<ProjectFile>> {
    paths
        .into_par_iter()
        .map(|(path, kind)| {
            let text = std::fs::read_to_string(&path)
                .with_context(|| format!("failed to read {}", path.display()))?;
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
        .filter_map(|result| match result {
            Ok(Some(file)) => Some(Ok(file)),
            Ok(None) => None,
            Err(err) => Some(Err(err)),
        })
        .collect()
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
    let relative = path
        .strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
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
    let relative = path
        .strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
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
            &DiscoveryOptions::with_ignore(vec![PathBuf::from("ignored")]),
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

        let result = discover(root, &[]).expect("discover");
        assert!(result.is_empty(), "{result:?}");
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
}
