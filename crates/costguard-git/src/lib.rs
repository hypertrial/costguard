use anyhow::{Context, Result};
use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};

#[derive(Debug)]
pub struct GitCommandError {
    pub command: String,
    pub status: i32,
    pub stderr: String,
}

impl fmt::Display for GitCommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "git {} failed with status {}: {}",
            self.command, self.status, self.stderr
        )
    }
}

impl Error for GitCommandError {}

impl GitCommandError {
    fn from_output(command: &str, status: ExitStatus, stderr: &[u8]) -> Self {
        Self {
            command: command.to_string(),
            status: status.code().unwrap_or(-1),
            stderr: String::from_utf8_lossy(stderr).trim().to_string(),
        }
    }
}

pub fn is_git_repository(root: &Path) -> bool {
    match run_git(root, &["rev-parse", "--is-inside-work-tree"]) {
        Ok(output) => {
            output.status.success() && String::from_utf8_lossy(&output.stdout).trim() == "true"
        }
        Err(_) => false,
    }
}

pub fn changed_files(root: &Path, base: &str) -> Result<Vec<PathBuf>> {
    if !is_git_repository(root) {
        anyhow::bail!("{} is not a git repository", root.display());
    }
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
    let command = format!("git {}", args.join(" "));
    let output = run_git(root, args).with_context(|| format!("failed to execute {command}"))?;
    if !output.status.success() {
        return Err(GitCommandError::from_output(&command, output.status, &output.stderr).into());
    }
    Ok(parse_paths(&output.stdout))
}

fn run_git(root: &Path, args: &[&str]) -> Result<std::process::Output> {
    Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .with_context(|| format!("failed to execute git {}", args.join(" ")))
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

    #[test]
    fn changed_files_fails_for_invalid_base_ref() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let root = tempdir.path();
        fs::write(root.join("model.sql"), "select 1\n").expect("write model");
        git(root, &["init"]);
        git(root, &["config", "user.email", "costguard@example.com"]);
        git(root, &["config", "user.name", "Costguard Test"]);
        git(root, &["add", "."]);
        git(root, &["commit", "-m", "initial"]);

        let err = changed_files(root, "does-not-exist").expect_err("invalid base");
        assert!(err.to_string().contains("does-not-exist"));
    }

    #[test]
    fn is_git_repository_false_for_plain_directory() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        assert!(!is_git_repository(tempdir.path()));
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
