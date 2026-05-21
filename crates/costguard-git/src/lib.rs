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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn changed_files_covers_committed_staged_unstaged_and_untracked_paths() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let root = tempdir.path();
        fs::create_dir_all(root.join("models")).expect("create models");
        fs::write(root.join("models/base.sql"), "select 1\n").expect("write base");

        git(root, &["init"]);
        git(root, &["checkout", "-b", "main"]);
        git(root, &["config", "user.email", "costguard@example.com"]);
        git(root, &["config", "user.name", "Costguard Test"]);
        git(root, &["add", "."]);
        git(root, &["commit", "-m", "initial"]);
        git(root, &["checkout", "-b", "feature"]);

        fs::write(root.join("models/committed.sql"), "select 2\n").expect("write committed");
        git(root, &["add", "."]);
        git(root, &["commit", "-m", "committed change"]);

        fs::write(root.join("models/staged.sql"), "select 3\n").expect("write staged");
        git(root, &["add", "models/staged.sql"]);
        fs::write(root.join("models/base.sql"), "select 4\n").expect("write unstaged");
        fs::write(root.join("models/untracked.sql"), "select 5\n").expect("write untracked");

        let changed = changed_files(root, "main").expect("changed files");
        assert!(changed.contains(&PathBuf::from("models/committed.sql")));
        assert!(changed.contains(&PathBuf::from("models/staged.sql")));
        assert!(changed.contains(&PathBuf::from("models/base.sql")));
        assert!(changed.contains(&PathBuf::from("models/untracked.sql")));
    }

    fn git(root: &Path, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(root)
            .output()
            .expect("run git");
        assert!(
            output.status.success(),
            "git {} failed\nstdout:\n{}\nstderr:\n{}",
            args.join(" "),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
