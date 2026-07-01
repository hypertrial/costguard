//! Git integration for PR-scoped scans.
//!
//! Detects changed files against a base branch (committed, staged, unstaged,
//! and untracked) for the `costguard pr` workflow.

use anyhow::{Context, Result};
use std::collections::{BTreeSet, HashMap};
use std::error::Error;
use std::ffi::OsString;
use std::fmt;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};

#[cfg(test)]
thread_local! {
    // ponytail: per-thread counter avoids parallel-test flake on a process-wide atomic
    static GIT_SPAWN_COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn spawn_git(root: &Path, args: &[&str]) -> Result<std::process::Child> {
    #[cfg(test)]
    GIT_SPAWN_COUNT.with(|count| count.set(count.get() + 1));
    Command::new("git")
        .args(args)
        .current_dir(root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("failed to execute git {}", args.join(" ")))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangedFileSets {
    pub base_commit: String,
    pub base: Vec<PathBuf>,
    pub head: Vec<PathBuf>,
}

impl ChangedFileSets {
    pub fn union(&self) -> Vec<PathBuf> {
        self.base
            .iter()
            .chain(&self.head)
            .cloned()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }
}

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

#[cfg(test)]
pub fn changed_files(root: &Path, base: &str) -> Result<Vec<PathBuf>> {
    Ok(changed_file_sets(root, base)?.union())
}

pub fn changed_file_sets(root: &Path, base: &str) -> Result<ChangedFileSets> {
    if !is_git_repository(root) {
        anyhow::bail!("{} is not a git repository", root.display());
    }
    let base_commit = resolve_base_commit(root, base)?;
    let output = run_git(root, &["diff", "--name-status", "-z", "-M", &base_commit])?;
    if !output.status.success() {
        return Err(GitCommandError::from_output(
            "git diff --name-status -z -M",
            output.status,
            &output.stderr,
        )
        .into());
    }
    let (base_paths, mut head_paths) = parse_name_status(&output.stdout)?;
    for path in git_paths(root, &["ls-files", "--others", "--exclude-standard", "-z"])? {
        head_paths.insert(path);
    }
    Ok(ChangedFileSets {
        base_commit,
        base: base_paths.into_iter().collect(),
        head: head_paths.into_iter().collect(),
    })
}

pub fn resolve_base_commit(root: &Path, base: &str) -> Result<String> {
    let output = run_git(root, &["merge-base", base, "HEAD"])?;
    if !output.status.success() {
        return Err(GitCommandError::from_output(
            &format!("git merge-base {base} HEAD"),
            output.status,
            &output.stderr,
        )
        .into());
    }
    let commit = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if commit.is_empty() {
        anyhow::bail!("git merge-base {base} HEAD returned no commit");
    }
    Ok(commit)
}

/// Returns file contents at `base:path`, or `None` when the path is new on HEAD.
#[cfg(test)]
pub fn file_at_ref(root: &Path, base: &str, path: &Path) -> Result<Option<String>> {
    if !is_git_repository(root) {
        anyhow::bail!("{} is not a git repository", root.display());
    }
    let spec = format!("{base}:{}", path.to_string_lossy());
    let output = run_git(root, &["show", &spec])?;
    if output.status.success() {
        return Ok(Some(String::from_utf8_lossy(&output.stdout).into_owned()));
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    if stderr.contains("exists on disk, but not in")
        || stderr.contains("does not exist")
        || stderr.contains("bad revision")
    {
        return Ok(None);
    }
    Err(
        GitCommandError::from_output(&format!("git show {spec}"), output.status, &output.stderr)
            .into(),
    )
}

pub fn files_at_commit_with_limit(
    root: &Path,
    commit: &str,
    paths: &[(PathBuf, u64)],
) -> Result<HashMap<PathBuf, Option<String>>> {
    if paths.is_empty() {
        return Ok(HashMap::new());
    }
    let specs = paths
        .iter()
        .map(|(path, _)| format!("{commit}:{}", path.to_string_lossy()))
        .collect::<Vec<_>>();
    let mut child = spawn_git(root, &["cat-file", "--batch", "-Z"])?;
    {
        let stdin = child
            .stdin
            .as_mut()
            .context("failed to open git cat-file stdin")?;
        for spec in &specs {
            stdin.write_all(spec.as_bytes())?;
            stdin.write_all(&[0])?;
        }
    }
    let output = child.wait_with_output()?;
    if !output.status.success() {
        return Err(GitCommandError::from_output(
            "git cat-file --batch -Z",
            output.status,
            &output.stderr,
        )
        .into());
    }

    let mut result = HashMap::new();
    let mut offset = 0usize;
    for (path, limit) in paths {
        let header_end = output.stdout[offset..]
            .iter()
            .position(|byte| *byte == 0)
            .with_context(|| format!("git cat-file truncated header for {}", path.display()))?
            + offset;
        let header = &output.stdout[offset..header_end];
        offset = header_end + 1;

        if header.ends_with(b" missing") {
            result.insert(path.clone(), None);
            continue;
        }

        let header = String::from_utf8_lossy(header);
        let mut fields = header.split_whitespace();
        let _object_name = fields.next().unwrap_or_default();
        let object_type = fields.next().unwrap_or_default();
        let observed = fields
            .next()
            .and_then(|value| value.parse::<u64>().ok())
            .with_context(|| format!("invalid git blob size for {}", path.display()))?;
        if object_type != "blob" {
            anyhow::bail!("git object for {} is not a file", path.display());
        }
        if observed > *limit {
            anyhow::bail!(
                "base file {} is {} bytes, exceeding configured limit of {} bytes",
                path.display(),
                observed,
                limit
            );
        }
        let size = observed as usize;
        let content_end = offset
            .checked_add(size)
            .context("git cat-file content overflow")?;
        if output.stdout.len() < content_end + 1 {
            anyhow::bail!(
                "git cat-file truncated content for {} (expected {} bytes)",
                path.display(),
                size
            );
        }
        let content = &output.stdout[offset..content_end];
        if output.stdout[content_end] != 0 {
            anyhow::bail!(
                "git cat-file content for {} is not NUL-terminated",
                path.display()
            );
        }
        offset = content_end + 1;
        let text = String::from_utf8(content.to_vec())
            .with_context(|| format!("base file {} is not valid UTF-8", path.display()))?;
        result.insert(path.clone(), Some(text));
    }
    if offset != output.stdout.len() {
        anyhow::bail!(
            "git cat-file returned {} trailing bytes after {} paths",
            output.stdout.len() - offset,
            paths.len()
        );
    }
    Ok(result)
}

fn parse_name_status(bytes: &[u8]) -> Result<(BTreeSet<PathBuf>, BTreeSet<PathBuf>)> {
    let fields = bytes
        .split(|byte| *byte == 0)
        .filter(|field| !field.is_empty())
        .collect::<Vec<_>>();
    let mut base = BTreeSet::new();
    let mut head = BTreeSet::new();
    let mut index = 0;
    while index < fields.len() {
        let status = fields[index];
        index += 1;
        let code = status.first().copied().unwrap_or_default();
        match code {
            b'R' => {
                let old = fields.get(index).context("rename missing old path")?;
                let new = fields.get(index + 1).context("rename missing new path")?;
                base.insert(path_from_git_bytes(old));
                head.insert(path_from_git_bytes(new));
                index += 2;
            }
            b'C' => {
                fields.get(index).context("copy missing old path")?;
                let new = fields.get(index + 1).context("copy missing new path")?;
                head.insert(path_from_git_bytes(new));
                index += 2;
            }
            b'A' => {
                let path = fields.get(index).context("addition missing path")?;
                head.insert(path_from_git_bytes(path));
                index += 1;
            }
            b'D' => {
                let path = fields.get(index).context("deletion missing path")?;
                base.insert(path_from_git_bytes(path));
                index += 1;
            }
            _ => {
                let path = fields.get(index).context("change missing path")?;
                let path = path_from_git_bytes(path);
                base.insert(path.clone());
                head.insert(path);
                index += 1;
            }
        }
    }
    Ok((base, head))
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
    bytes
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .map(path_from_git_bytes)
        .collect()
}

#[cfg(unix)]
fn path_from_git_bytes(bytes: &[u8]) -> PathBuf {
    use std::os::unix::ffi::OsStringExt;
    PathBuf::from(OsString::from_vec(bytes.to_vec()))
}

#[cfg(not(unix))]
fn path_from_git_bytes(bytes: &[u8]) -> PathBuf {
    PathBuf::from(String::from_utf8_lossy(bytes).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn file_at_ref_reads_committed_base_content() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let root = tempdir.path();
        fs::create_dir_all(root.join("models")).expect("create models");
        fs::write(root.join("models/a.sql"), "select 1\n").expect("write");
        git(root, &["init"]);
        git(root, &["checkout", "-b", "main"]);
        git(root, &["config", "user.email", "costguard@example.com"]);
        git(root, &["config", "user.name", "Costguard Test"]);
        git(root, &["add", "."]);
        git(root, &["commit", "-m", "initial"]);
        git(root, &["checkout", "-b", "feature"]);
        fs::write(root.join("models/a.sql"), "select 2\n").expect("write");
        let content = file_at_ref(root, "main", Path::new("models/a.sql")).expect("base content");
        assert_eq!(content.as_deref(), Some("select 1\n"));
        assert!(file_at_ref(root, "main", Path::new("models/new.sql"))
            .expect("missing")
            .is_none());
    }

    #[test]
    fn changed_files_covers_committed_staged_unstaged_and_untracked_paths() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let root = tempdir.path();
        fs::create_dir_all(root.join("models")).expect("create models");
        fs::write(root.join("models/base.sql"), "select 1\n").expect("write base");
        fs::write(root.join("models/deleted.sql"), "select 0\n").expect("write deleted");

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
        fs::remove_file(root.join("models/deleted.sql")).expect("delete tracked");
        fs::write(root.join("models/untracked.sql"), "select 5\n").expect("write untracked");

        let sets = changed_file_sets(root, "main").expect("changed file sets");
        assert!(sets.base.contains(&PathBuf::from("models/base.sql")));
        assert!(sets.head.contains(&PathBuf::from("models/base.sql")));
        assert!(sets.base.contains(&PathBuf::from("models/deleted.sql")));
        assert!(!sets.head.contains(&PathBuf::from("models/deleted.sql")));
        for added in ["committed.sql", "staged.sql", "untracked.sql"] {
            let path = PathBuf::from("models").join(added);
            assert!(sets.head.contains(&path));
            assert!(!sets.base.contains(&path));
        }

        let changed = changed_files(root, "main").expect("changed files");
        assert!(changed.contains(&PathBuf::from("models/committed.sql")));
        assert!(changed.contains(&PathBuf::from("models/staged.sql")));
        assert!(changed.contains(&PathBuf::from("models/base.sql")));
        assert!(changed.contains(&PathBuf::from("models/untracked.sql")));
        assert!(changed.contains(&PathBuf::from("models/deleted.sql")));
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
    fn parse_paths_preserves_whitespace_and_newlines() {
        let paths =
            parse_paths(b"models/ leading.sql\0models/tab\tname.sql\0models/new\nline.sql\0");
        assert_eq!(
            paths,
            vec![
                PathBuf::from("models/ leading.sql"),
                PathBuf::from("models/tab\tname.sql"),
                PathBuf::from("models/new\nline.sql"),
            ]
        );
    }

    #[test]
    fn changed_files_preserves_unusual_paths_and_renames() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let root = tempdir.path();
        fs::create_dir_all(root.join("models")).expect("create models");
        for name in [
            " leading.sql",
            "tab\tname.sql",
            "new\nline.sql",
            "unicodé.sql",
            "-dash.sql",
        ] {
            fs::write(root.join("models").join(name), "select 1\n").expect("write model");
        }
        fs::write(root.join("models/rename me.sql"), "select 1\n").expect("write rename source");
        git(root, &["init"]);
        git(root, &["checkout", "-b", "main"]);
        git(root, &["config", "user.email", "costguard@example.com"]);
        git(root, &["config", "user.name", "Costguard Test"]);
        git(root, &["add", "."]);
        git(root, &["commit", "-m", "initial"]);
        git(root, &["checkout", "-b", "feature"]);

        for name in [
            " leading.sql",
            "tab\tname.sql",
            "new\nline.sql",
            "unicodé.sql",
            "-dash.sql",
        ] {
            fs::write(root.join("models").join(name), "select 2\n").expect("modify model");
        }
        git(root, &["mv", "models/rename me.sql", "models/renamed.sql"]);
        fs::write(root.join("models/untracked\nmodel.sql"), "select 3\n").expect("write untracked");

        let sets = changed_file_sets(root, "main").expect("changed file sets");
        assert!(sets.base.contains(&PathBuf::from("models/rename me.sql")));
        assert!(!sets.head.contains(&PathBuf::from("models/rename me.sql")));
        assert!(sets.head.contains(&PathBuf::from("models/renamed.sql")));
        assert!(!sets.base.contains(&PathBuf::from("models/renamed.sql")));
        assert!(sets
            .head
            .contains(&PathBuf::from("models/untracked\nmodel.sql")));
        assert!(!sets
            .base
            .contains(&PathBuf::from("models/untracked\nmodel.sql")));

        let changed = changed_files(root, "main").expect("changed files");
        for name in [
            " leading.sql",
            "tab\tname.sql",
            "new\nline.sql",
            "unicodé.sql",
            "-dash.sql",
            "renamed.sql",
            "untracked\nmodel.sql",
        ] {
            assert!(
                changed.contains(&PathBuf::from("models").join(name)),
                "missing {name:?}: {changed:?}"
            );
        }
    }

    #[test]
    fn is_git_repository_false_for_plain_directory() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        assert!(!is_git_repository(tempdir.path()));
    }

    #[test]
    fn blob_size_is_checked_before_base_content_is_loaded() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let root = tempdir.path();
        fs::write(root.join("model.sql"), "12345").expect("write model");
        git(root, &["init"]);
        git(root, &["config", "user.email", "costguard@example.com"]);
        git(root, &["config", "user.name", "Costguard Test"]);
        git(root, &["add", "."]);
        git(root, &["commit", "-m", "initial"]);
        let commit = resolve_base_commit(root, "HEAD").expect("base commit");

        let error = files_at_commit_with_limit(root, &commit, &[(PathBuf::from("model.sql"), 4)])
            .expect_err("oversized blob");
        let message = error.to_string();
        assert!(message.contains("model.sql"));
        assert!(message.contains("5 bytes"));
        assert!(message.contains("4 bytes"));

        let files = files_at_commit_with_limit(root, &commit, &[(PathBuf::from("model.sql"), 5)])
            .expect("blob at limit");
        assert_eq!(files[Path::new("model.sql")].as_deref(), Some("12345"));
    }

    #[test]
    fn batch_load_does_not_spawn_per_file_git_commands() {
        GIT_SPAWN_COUNT.with(|count| count.set(0));
        let tempdir = tempfile::tempdir().expect("tempdir");
        let root = tempdir.path();
        fs::create_dir_all(root.join("models")).expect("models dir");
        for index in 0..20 {
            fs::write(
                root.join("models").join(format!("m{index}.sql")),
                format!("select {index}\n"),
            )
            .expect("write model");
        }
        git(root, &["init"]);
        git(root, &["config", "user.email", "costguard@example.com"]);
        git(root, &["config", "user.name", "Costguard Test"]);
        git(root, &["add", "."]);
        git(root, &["commit", "-m", "initial"]);
        let commit = resolve_base_commit(root, "HEAD").expect("base commit");

        let requests = (0..20)
            .map(|index| (PathBuf::from(format!("models/m{index}.sql")), 1024u64))
            .collect::<Vec<_>>();
        GIT_SPAWN_COUNT.with(|count| count.set(0));
        let files = files_at_commit_with_limit(root, &commit, &requests).expect("batch load");
        assert_eq!(files.len(), 20);
        GIT_SPAWN_COUNT.with(|count| {
            assert_eq!(
                count.get(),
                1,
                "expected one git subprocess for batch blob load"
            );
        });
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
