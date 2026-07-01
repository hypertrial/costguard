#![allow(dead_code)] // ponytail: each integration test binary includes this module independently; not all helpers used in every binary.

use std::path::PathBuf;
use std::process::Command;

pub fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_costguard"))
}

pub fn costguard_command() -> Command {
    let mut command = Command::new(bin());
    command.current_dir(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."));
    command
}

pub fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
}

pub fn keygen(
    private_key: &std::path::Path,
    trust_store: &std::path::Path,
    force: bool,
) -> std::process::Output {
    let mut command = costguard_command();
    command
        .args(["policy", "keygen", "root-2026", "--private-key"])
        .arg(private_key)
        .arg("--trust-store")
        .arg(trust_store);
    if force {
        command.arg("--force");
    }
    command.output().expect("run keygen")
}

pub fn git(root: &std::path::Path, args: &[&str]) {
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
