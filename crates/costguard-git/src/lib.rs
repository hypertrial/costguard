use anyhow::{Context, Result};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn changed_files(root: &Path, base: &str) -> Result<Vec<PathBuf>> {
    let mut paths = BTreeSet::new();
    for path in committed_diff(root, base)? {
        paths.insert(path);
    }
    for args in [
        &["diff", "--name-only"][..],
        &["diff", "--cached", "--name-only"][..],
        &["ls-files", "--others", "--exclude-standard"][..],
    ] {
        for path in git_paths(root, args)? {
            paths.insert(path);
        }
    }
    Ok(paths.into_iter().collect())
}

fn committed_diff(root: &Path, base: &str) -> Result<Vec<PathBuf>> {
    let range = format!("{base}...HEAD");
    let merge_base_args = ["diff", "--name-only", range.as_str()];
    let paths = git_paths(root, &merge_base_args)?;
    if paths.is_empty() {
        git_paths(root, &["diff", "--name-only", base])
    } else {
        Ok(paths)
    }
}

fn git_paths(root: &Path, args: &[&str]) -> Result<Vec<PathBuf>> {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .with_context(|| format!("failed to execute git {}", args.join(" ")))?;
    if !output.status.success() {
        return Ok(Vec::new());
    }
    Ok(parse_paths(&output.stdout))
}

fn parse_paths(bytes: &[u8]) -> Vec<PathBuf> {
    String::from_utf8_lossy(bytes)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(PathBuf::from)
        .collect()
}
