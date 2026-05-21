use anyhow::{Context, Result};
use costguard_diagnostics::LineIndex;
use ignore::WalkBuilder;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

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

#[derive(Debug, Clone, Default)]
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
    for scan_root in scan_roots {
        for entry in WalkBuilder::new(&scan_root)
            .standard_filters(true)
            .hidden(false)
            .build()
        {
            let entry = entry?;
            if !entry.file_type().is_some_and(|kind| kind.is_file()) {
                continue;
            }
            let path = entry.into_path();
            let kind = classify(&path, root);
            if kind != FileKind::Other {
                paths.push((path, kind));
            }
        }
    }
    paths.sort_by(|left, right| left.0.cmp(&right.0));
    paths.dedup_by(|left, right| left.0 == right.0);
    load_project_files(root, paths)
}

pub fn read_existing_paths(root: &Path, requested_paths: &[PathBuf]) -> Result<Vec<ProjectFile>> {
    let mut paths = requested_paths
        .iter()
        .map(|path| {
            if path.is_absolute() {
                path.clone()
            } else {
                root.join(path)
            }
        })
        .filter(|path| path.is_file())
        .filter_map(|path| {
            let kind = classify(&path, root);
            (kind != FileKind::Other).then_some((path, kind))
        })
        .collect::<Vec<_>>();
    paths.sort_by(|left, right| left.0.cmp(&right.0));
    paths.dedup_by(|left, right| left.0 == right.0);
    load_project_files(root, paths)
}

fn load_project_files(root: &Path, paths: Vec<(PathBuf, FileKind)>) -> Result<Vec<ProjectFile>> {
    paths
        .into_par_iter()
        .map(|(path, kind)| {
            let text = std::fs::read_to_string(&path)
                .with_context(|| format!("failed to read {}", path.display()))?;
            let root_relative_path = path.strip_prefix(root).unwrap_or(&path).to_path_buf();
            let line_index = LineIndex::new(&text);
            Ok(ProjectFile {
                path,
                root_relative_path,
                kind,
                text,
                line_index,
            })
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
        "sql" if relative.contains("models/") => FileKind::DbtSqlModel,
        "sql" => FileKind::Sql,
        "yml" | "yaml" => FileKind::DbtYaml,
        "py" => FileKind::Python,
        _ => FileKind::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_dbt_sql() {
        assert_eq!(
            classify(Path::new("/r/models/marts/fct.sql"), Path::new("/r")),
            FileKind::DbtSqlModel
        );
    }
}
