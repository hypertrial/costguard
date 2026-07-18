//! Git integration for PR-scoped scans.
//!
//! Detects changed files against a base branch (committed, staged, unstaged,
//! and untracked) for the `costguard pr` workflow.

use anyhow::{Context, Result};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::error::Error;
use std::ffi::OsString;
use std::fmt;
use std::io::{BufRead, BufReader, Read, Write};
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

pub(crate) fn head_commit(root: &Path) -> Result<String> {
    let output = run_git(root, &["rev-parse", "--verify", "HEAD"])?;
    if !output.status.success() {
        return Err(GitCommandError::from_output(
            "git rev-parse --verify HEAD",
            output.status,
            &output.stderr,
        )
        .into());
    }
    let commit = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if commit.is_empty() {
        anyhow::bail!("git rev-parse --verify HEAD returned no commit");
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

#[allow(dead_code)] // compatibility wrapper for callers that only need per-file limits
pub fn files_at_commit_with_limit(
    root: &Path,
    commit: &str,
    paths: &[(PathBuf, u64)],
) -> Result<HashMap<PathBuf, Option<String>>> {
    files_at_commit_with_total_limit(root, commit, paths, u64::MAX)
}

pub(crate) fn files_at_commit_with_total_limit(
    root: &Path,
    commit: &str,
    paths: &[(PathBuf, u64)],
    max_total_bytes: u64,
) -> Result<HashMap<PathBuf, Option<String>>> {
    files_at_commit_with_budget(root, commit, paths, 0, max_total_bytes)
}

pub(crate) fn files_at_commit_with_budget(
    root: &Path,
    commit: &str,
    paths: &[(PathBuf, u64)],
    initial_total_bytes: u64,
    max_total_bytes: u64,
) -> Result<HashMap<PathBuf, Option<String>>> {
    files_at_commit_bytes_with_budget(root, commit, paths, initial_total_bytes, max_total_bytes)?
        .into_iter()
        .map(|(path, content)| {
            let text = content
                .map(String::from_utf8)
                .transpose()
                .with_context(|| format!("base file {} is not valid UTF-8", path.display()))?;
            Ok((path, text))
        })
        .collect()
}

pub(crate) fn files_at_commit_bytes_with_budget(
    root: &Path,
    commit: &str,
    paths: &[(PathBuf, u64)],
    initial_total_bytes: u64,
    max_total_bytes: u64,
) -> Result<HashMap<PathBuf, Option<Vec<u8>>>> {
    if paths.is_empty() {
        return Ok(HashMap::new());
    }
    let specs = object_specs_at_commit(root, commit, paths)?;
    let mut child = spawn_git(root, &["cat-file", "--batch-check", "-Z"])?;
    let writer = write_batch_specs_async(&mut child, specs.clone())?;
    let output = child.wait_with_output()?;
    finish_batch_writer(writer)?;
    if !output.status.success() {
        return Err(GitCommandError::from_output(
            "git cat-file --batch-check -Z",
            output.status,
            &output.stderr,
        )
        .into());
    }

    let mut approved = Vec::new();
    let mut result = HashMap::with_capacity(paths.len());
    let mut offset = 0usize;
    let mut total_bytes = initial_total_bytes;
    for ((path, limit), spec) in paths.iter().zip(&specs) {
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

        let (object_type, observed) = parse_batch_header(header, path)?;
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
        total_bytes = total_bytes
            .checked_add(observed)
            .context("base file aggregate size overflow")?;
        if total_bytes > max_total_bytes {
            anyhow::bail!(
                "base snapshot is {} bytes, exceeding configured aggregate limit of {} bytes",
                total_bytes,
                max_total_bytes
            );
        }
        approved.push((path, spec, observed));
    }
    if offset != output.stdout.len() {
        anyhow::bail!(
            "git cat-file --batch-check returned {} trailing bytes after {} paths",
            output.stdout.len() - offset,
            paths.len()
        );
    }

    if approved.is_empty() {
        return Ok(result);
    }
    let approved_specs = approved
        .iter()
        .map(|(_, spec, _)| (*spec).clone())
        .collect::<Vec<_>>();
    let mut child = spawn_git(root, &["cat-file", "--batch", "-Z"])?;
    let writer = write_batch_specs_async(&mut child, approved_specs)?;
    let stdout = child
        .stdout
        .take()
        .context("failed to open git cat-file stdout")?;
    let mut reader = BufReader::new(stdout);
    let read_result = (|| -> Result<()> {
        for (path, _, expected_size) in approved {
            let header = read_nul_field(&mut reader, path)?;
            let (object_type, observed) = parse_batch_header(&header, path)?;
            if object_type != "blob" || observed != expected_size {
                anyhow::bail!(
                    "git object for {} changed after size preflight",
                    path.display()
                );
            }
            let size = usize::try_from(observed).context("git blob is too large to read")?;
            let mut content = vec![0u8; size];
            reader.read_exact(&mut content).with_context(|| {
                format!(
                    "git cat-file truncated content for {} (expected {} bytes)",
                    path.display(),
                    size
                )
            })?;
            let mut terminator = [0u8; 1];
            reader.read_exact(&mut terminator).with_context(|| {
                format!(
                    "git cat-file content for {} has no terminator",
                    path.display()
                )
            })?;
            if terminator[0] != 0 {
                anyhow::bail!(
                    "git cat-file content for {} is not NUL-terminated",
                    path.display()
                );
            }
            result.insert(path.clone(), Some(content));
        }
        let mut trailing = [0u8; 1];
        if reader.read(&mut trailing)? != 0 {
            anyhow::bail!("git cat-file returned trailing bytes after approved paths");
        }
        Ok(())
    })();
    drop(reader);
    if read_result.is_err() {
        let _ = child.kill();
    }
    let write_result = finish_batch_writer(writer);
    let output = child.wait_with_output()?;
    if !output.status.success() {
        return Err(GitCommandError::from_output(
            "git cat-file --batch -Z",
            output.status,
            &output.stderr,
        )
        .into());
    }
    write_result?;
    read_result?;
    Ok(result)
}

#[derive(Debug)]
pub(crate) struct BaseObjectStore {
    contents: HashMap<PathBuf, Option<Vec<u8>>>,
}

impl BaseObjectStore {
    pub(crate) fn load(
        root: &Path,
        commit: &str,
        paths: impl IntoIterator<Item = (PathBuf, u64)>,
        initial_total_bytes: u64,
        max_total_bytes: u64,
    ) -> Result<Self> {
        if initial_total_bytes > max_total_bytes {
            anyhow::bail!(
                "base snapshot is {} bytes, exceeding configured aggregate limit of {} bytes",
                initial_total_bytes,
                max_total_bytes
            );
        }
        let mut limits = BTreeMap::<PathBuf, u64>::new();
        for (path, limit) in paths {
            limits
                .entry(path)
                .and_modify(|existing| *existing = (*existing).min(limit))
                .or_insert(limit);
        }
        let requests = limits.into_iter().collect::<Vec<_>>();
        Ok(Self {
            contents: files_at_commit_bytes_with_budget(
                root,
                commit,
                &requests,
                initial_total_bytes,
                max_total_bytes,
            )?,
        })
    }

    pub(crate) fn bytes(&self, path: &Path) -> Option<&[u8]> {
        self.contents.get(path).and_then(Option::as_deref)
    }

    pub(crate) fn text(&self, path: &Path) -> Result<Option<&str>> {
        self.bytes(path)
            .map(|bytes| {
                std::str::from_utf8(bytes)
                    .with_context(|| format!("base file {} is not valid UTF-8", path.display()))
            })
            .transpose()
    }
}

fn write_batch_specs_async(
    child: &mut std::process::Child,
    specs: Vec<OsString>,
) -> Result<std::thread::JoinHandle<Result<()>>> {
    let mut stdin = child
        .stdin
        .take()
        .context("failed to open git cat-file stdin")?;
    Ok(std::thread::spawn(move || {
        for spec in specs {
            stdin.write_all(spec.as_os_str().as_encoded_bytes())?;
            stdin.write_all(&[0])?;
        }
        Ok(())
    }))
}

fn finish_batch_writer(writer: std::thread::JoinHandle<Result<()>>) -> Result<()> {
    writer
        .join()
        .map_err(|_| anyhow::anyhow!("git cat-file input writer panicked"))?
}

fn object_specs_at_commit(
    root: &Path,
    commit: &str,
    paths: &[(PathBuf, u64)],
) -> Result<Vec<OsString>> {
    const MAX_PATHSPEC_BYTES: usize = 128 * 1024;
    const MAX_PATHS_PER_QUERY: usize = 512;

    let mut object_ids = HashMap::with_capacity(paths.len());
    let mut chunk = Vec::new();
    let mut chunk_bytes = 0usize;
    for (path, _) in paths {
        let path_bytes = path.as_os_str().as_encoded_bytes().len();
        if path.as_os_str().is_empty() || path_bytes > MAX_PATHSPEC_BYTES {
            continue;
        }
        if !chunk.is_empty()
            && (chunk.len() == MAX_PATHS_PER_QUERY
                || chunk_bytes.saturating_add(path_bytes) > MAX_PATHSPEC_BYTES)
        {
            load_object_ids(root, commit, &chunk, &mut object_ids)?;
            chunk.clear();
            chunk_bytes = 0;
        }
        chunk.push(path.as_path());
        chunk_bytes = chunk_bytes.saturating_add(path_bytes);
    }
    if !chunk.is_empty() {
        load_object_ids(root, commit, &chunk, &mut object_ids)?;
    }

    Ok(paths
        .iter()
        .map(|(path, _)| {
            object_ids
                .get(path)
                .cloned()
                .unwrap_or_else(|| commit_path_spec(commit, path))
        })
        .collect())
}

fn load_object_ids(
    root: &Path,
    commit: &str,
    paths: &[&Path],
    object_ids: &mut HashMap<PathBuf, OsString>,
) -> Result<()> {
    let pathspecs = paths
        .iter()
        .map(|path| {
            let mut pathspec = OsString::from(":(literal)");
            pathspec.push(path);
            pathspec
        })
        .collect::<Vec<_>>();
    let output = Command::new("git")
        .args(["ls-tree", "-z", "--full-tree", commit, "--"])
        .args(&pathspecs)
        .current_dir(root)
        .output()
        .with_context(|| format!("failed to execute git ls-tree for {commit}"))?;
    if !output.status.success() {
        return Err(GitCommandError::from_output(
            "git ls-tree -z --full-tree",
            output.status,
            &output.stderr,
        )
        .into());
    }

    let wanted = paths.iter().copied().collect::<BTreeSet<_>>();
    for record in output.stdout.split(|byte| *byte == 0) {
        if record.is_empty() {
            continue;
        }
        let separator = record
            .iter()
            .position(|byte| *byte == b'\t')
            .context("git ls-tree returned a malformed record")?;
        let metadata = std::str::from_utf8(&record[..separator])
            .context("git ls-tree returned invalid metadata")?;
        let object_id = metadata
            .split_whitespace()
            .nth(2)
            .context("git ls-tree record is missing an object ID")?;
        let path = path_from_git_bytes(&record[separator + 1..]);
        if wanted.contains(path.as_path()) {
            object_ids.insert(path, OsString::from(object_id));
        }
    }
    Ok(())
}

fn commit_path_spec(commit: &str, path: &Path) -> OsString {
    let mut spec = OsString::from(commit);
    spec.push(":");
    spec.push(path);
    spec
}

fn read_nul_field(reader: &mut impl BufRead, path: &Path) -> Result<Vec<u8>> {
    let mut field = Vec::new();
    let read = reader.read_until(0, &mut field)?;
    if read == 0 || field.pop() != Some(0) {
        anyhow::bail!("git cat-file truncated header for {}", path.display());
    }
    Ok(field)
}

fn parse_batch_header<'a>(header: &'a [u8], path: &Path) -> Result<(&'a str, u64)> {
    let header = std::str::from_utf8(header)
        .with_context(|| format!("invalid git blob header for {}", path.display()))?;
    let mut fields = header.split_whitespace();
    let _object_name = fields.next().unwrap_or_default();
    let object_type = fields.next().unwrap_or_default();
    let observed = fields
        .next()
        .and_then(|value| value.parse::<u64>().ok())
        .with_context(|| format!("invalid git blob size for {}", path.display()))?;
    Ok((object_type, observed))
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
                2,
                "expected one preflight and one content subprocess"
            );
        });
    }

    #[test]
    fn batch_load_drains_output_while_writing_more_than_pipe_capacity() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let root = tempdir.path();
        fs::write(root.join("model.sql"), "select 1\n").expect("write model");
        git(root, &["init"]);
        git(root, &["config", "user.email", "costguard@example.com"]);
        git(root, &["config", "user.name", "Costguard Test"]);
        git(root, &["add", "."]);
        git(root, &["commit", "-m", "initial"]);
        let commit = resolve_base_commit(root, "HEAD").expect("base commit");
        let requests = vec![(PathBuf::from("model.sql"), 1024); 5_000];

        let files = files_at_commit_with_total_limit(root, &commit, &requests, 100_000)
            .expect("batch load");

        assert_eq!(files[Path::new("model.sql")].as_deref(), Some("select 1\n"));
    }

    #[test]
    fn aggregate_limit_is_checked_before_content_is_loaded() {
        GIT_SPAWN_COUNT.with(|count| count.set(0));
        let tempdir = tempfile::tempdir().expect("tempdir");
        let root = tempdir.path();
        fs::write(root.join("a.sql"), "123").expect("write a");
        fs::write(root.join("b.sql"), "456").expect("write b");
        git(root, &["init"]);
        git(root, &["config", "user.email", "costguard@example.com"]);
        git(root, &["config", "user.name", "Costguard Test"]);
        git(root, &["add", "."]);
        git(root, &["commit", "-m", "initial"]);
        let commit = resolve_base_commit(root, "HEAD").expect("base commit");

        GIT_SPAWN_COUNT.with(|count| count.set(0));
        let error = files_at_commit_with_total_limit(
            root,
            &commit,
            &[(PathBuf::from("a.sql"), 3), (PathBuf::from("b.sql"), 3)],
            5,
        )
        .expect_err("aggregate limit");
        assert!(error.to_string().contains("aggregate limit"), "{error:#}");
        GIT_SPAWN_COUNT.with(|count| {
            assert_eq!(count.get(), 1, "content process must not start");
        });
    }

    #[test]
    fn aggregate_limit_includes_preflighted_local_manifest_bytes() {
        GIT_SPAWN_COUNT.with(|count| count.set(0));
        let tempdir = tempfile::tempdir().expect("tempdir");
        let root = tempdir.path();
        fs::write(root.join("model.sql"), "123").expect("write model");
        git(root, &["init"]);
        git(root, &["config", "user.email", "costguard@example.com"]);
        git(root, &["config", "user.name", "Costguard Test"]);
        git(root, &["add", "."]);
        git(root, &["commit", "-m", "initial"]);
        let commit = resolve_base_commit(root, "HEAD").expect("base commit");

        let error =
            files_at_commit_with_budget(root, &commit, &[(PathBuf::from("model.sql"), 3)], 3, 5)
                .expect_err("manifest plus source must exceed aggregate limit");

        assert!(error.to_string().contains("aggregate limit"), "{error:#}");
        GIT_SPAWN_COUNT.with(|count| {
            assert_eq!(count.get(), 1, "content process must not start");
        });
    }

    #[test]
    fn base_object_store_deduplicates_paths_and_uses_strictest_limit() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let root = tempdir.path();
        fs::write(root.join("model.sql"), "123").expect("write model");
        git(root, &["init"]);
        git(root, &["config", "user.email", "costguard@example.com"]);
        git(root, &["config", "user.name", "Costguard Test"]);
        git(root, &["add", "."]);
        git(root, &["commit", "-m", "initial"]);
        let commit = resolve_base_commit(root, "HEAD").expect("base commit");

        let store = BaseObjectStore::load(
            root,
            &commit,
            [
                (PathBuf::from("model.sql"), 3),
                (PathBuf::from("model.sql"), 4),
            ],
            0,
            3,
        )
        .expect("duplicate path counts once");
        assert_eq!(store.text(Path::new("model.sql")).unwrap(), Some("123"));

        let error = BaseObjectStore::load(
            root,
            &commit,
            [
                (PathBuf::from("model.sql"), 3),
                (PathBuf::from("model.sql"), 2),
            ],
            0,
            3,
        )
        .expect_err("strictest per-file limit must win");
        assert!(error.to_string().contains("configured limit of 2"));
    }

    #[test]
    fn base_object_store_enforces_one_budget_across_all_consumers() {
        GIT_SPAWN_COUNT.with(|count| count.set(0));
        let tempdir = tempfile::tempdir().expect("tempdir");
        let root = tempdir.path();
        fs::create_dir(root.join("models")).expect("create models");
        fs::write(root.join("manifest.json"), "123").expect("write manifest");
        fs::write(root.join("models/orders.sql"), "456").expect("write model");
        fs::write(root.join("rocky.toml"), "789").expect("write Rocky config");
        git(root, &["init"]);
        git(root, &["config", "user.email", "costguard@example.com"]);
        git(root, &["config", "user.name", "Costguard Test"]);
        git(root, &["add", "."]);
        git(root, &["commit", "-m", "initial"]);
        let commit = resolve_base_commit(root, "HEAD").expect("base commit");

        GIT_SPAWN_COUNT.with(|count| count.set(0));
        let error = BaseObjectStore::load(
            root,
            &commit,
            [
                (PathBuf::from("models/orders.sql"), 3),
                (PathBuf::from("models/orders.sql"), 4),
                (PathBuf::from("rocky.toml"), 3),
            ],
            3,
            8,
        )
        .expect_err("manifest, model, and Rocky inputs share one aggregate budget");

        assert!(error.to_string().contains("aggregate limit"), "{error:#}");
        GIT_SPAWN_COUNT.with(|count| {
            assert_eq!(count.get(), 1, "content process must not start");
        });
    }

    #[test]
    fn batch_load_preserves_missing_and_unusual_paths() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let root = tempdir.path();
        let unusual = PathBuf::from("new\nline.sql");
        fs::write(root.join(&unusual), "select 1\n").expect("write unusual path");
        git(root, &["init"]);
        git(root, &["config", "user.email", "costguard@example.com"]);
        git(root, &["config", "user.name", "Costguard Test"]);
        git(root, &["add", "."]);
        git(root, &["commit", "-m", "initial"]);
        let commit = resolve_base_commit(root, "HEAD").expect("base commit");

        let files = files_at_commit_with_total_limit(
            root,
            &commit,
            &[
                (unusual.clone(), 1024),
                (PathBuf::from("missing.sql"), 1024),
            ],
            1024,
        )
        .expect("batch load");

        assert_eq!(files[&unusual].as_deref(), Some("select 1\n"));
        assert_eq!(files[Path::new("missing.sql")], None);
    }

    #[test]
    fn batch_load_treats_git_pathspec_metacharacters_literally() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let root = tempdir.path();
        let literal = PathBuf::from("models/[a].sql");
        fs::create_dir_all(root.join("models")).expect("create models");
        fs::write(root.join(&literal), "select 'literal'\n").expect("write literal path");
        fs::write(root.join("models/a.sql"), "select 'glob match'\n").expect("write glob path");
        git(root, &["init"]);
        git(root, &["config", "user.email", "costguard@example.com"]);
        git(root, &["config", "user.name", "Costguard Test"]);
        git(root, &["add", "."]);
        git(root, &["commit", "-m", "initial"]);
        let commit = resolve_base_commit(root, "HEAD").expect("base commit");

        let files =
            files_at_commit_with_total_limit(root, &commit, &[(literal.clone(), 1024)], 1024)
                .expect("load literal path");

        assert_eq!(files.len(), 1);
        assert_eq!(files[&literal].as_deref(), Some("select 'literal'\n"));
    }

    #[test]
    fn batch_preflight_rejects_non_blob_objects() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let root = tempdir.path();
        fs::write(root.join("model.sql"), "select 1\n").expect("write model");
        git(root, &["init"]);
        git(root, &["config", "user.email", "costguard@example.com"]);
        git(root, &["config", "user.name", "Costguard Test"]);
        git(root, &["add", "."]);
        git(root, &["commit", "-m", "initial"]);
        let commit = resolve_base_commit(root, "HEAD").expect("base commit");

        let error =
            files_at_commit_with_total_limit(root, &commit, &[(PathBuf::new(), 1024)], 1024)
                .expect_err("tree must be rejected");

        assert!(error.to_string().contains("not a file"), "{error:#}");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn batch_load_preserves_non_utf8_paths() {
        use std::os::unix::ffi::OsStringExt;

        let tempdir = tempfile::tempdir().expect("tempdir");
        let root = tempdir.path();
        let path = PathBuf::from(OsString::from_vec(b"model-\xff.sql".to_vec()));
        fs::write(root.join(&path), "select 1\n").expect("write non-UTF-8 path");
        git(root, &["init"]);
        git(root, &["config", "user.email", "costguard@example.com"]);
        git(root, &["config", "user.name", "Costguard Test"]);
        git(root, &["add", "."]);
        git(root, &["commit", "-m", "initial"]);
        let commit = resolve_base_commit(root, "HEAD").expect("base commit");

        let files = files_at_commit_with_total_limit(root, &commit, &[(path.clone(), 1024)], 1024)
            .expect("load non-UTF-8 path");

        assert_eq!(files[&path].as_deref(), Some("select 1\n"));
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
