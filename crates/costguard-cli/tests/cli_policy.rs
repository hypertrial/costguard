mod common;

use common::{costguard_command, keygen};
use std::fs;

#[test]
fn policy_cli_compiles_signs_and_verifies() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("policy.toml");
    let compiled = temp.path().join("policy.json");
    let private_key = temp.path().join("private.json");
    let trust = temp.path().join("trust.json");
    let bundle = temp.path().join("bundle.json");
    let now = chrono::Utc::now();
    fs::write(
        &source,
        format!(
            r#"schema_version = 2
id = "org-default"
version = "2026.06"
organization = "acme"
issued_at = "{}"
expires_at = "{}"
identity_scheme = "semantic-v1"

[[scopes]]
id = "org"
kind = "organization"
selector = "acme"
priority = 0
enforcement = "block"
"#,
            (now - chrono::Duration::minutes(1)).to_rfc3339(),
            (now + chrono::Duration::days(30)).to_rfc3339()
        ),
    )
    .unwrap();

    let keygen = costguard_command()
        .args(["policy", "keygen", "root-2026", "--private-key"])
        .arg(&private_key)
        .arg("--trust-store")
        .arg(&trust)
        .output()
        .unwrap();
    assert!(
        keygen.status.success(),
        "{}",
        String::from_utf8_lossy(&keygen.stderr)
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&private_key).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    let commands = [
        vec![source.clone(), compiled.clone()],
        vec![compiled.clone(), private_key.clone(), bundle.clone()],
        vec![bundle.clone(), trust.clone()],
    ];
    for (subcommand, paths) in ["compile", "sign", "verify"].into_iter().zip(commands) {
        let output = costguard_command()
            .args(["policy", subcommand])
            .args(paths)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn policy_keygen_refuses_existing_outputs_without_force() {
    let temp = tempfile::tempdir().unwrap();
    let private = temp.path().join("private.json");
    let trust = temp.path().join("trust.json");
    fs::write(&private, "keep-private").unwrap();
    fs::write(&trust, "keep-trust").unwrap();
    let output = keygen(&private, &trust, false);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("--force"));
    assert_eq!(fs::read_to_string(private).unwrap(), "keep-private");
    assert_eq!(fs::read_to_string(trust).unwrap(), "keep-trust");
}

#[test]
fn policy_keygen_force_replaces_regular_files() {
    let temp = tempfile::tempdir().unwrap();
    let private = temp.path().join("private.json");
    let trust = temp.path().join("trust.json");
    fs::write(&private, "old-private").unwrap();
    fs::write(&trust, "old-trust").unwrap();
    let output = keygen(&private, &trust, true);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(fs::read_to_string(private).unwrap().contains("private_key"));
    assert!(fs::read_to_string(trust).unwrap().contains("public_key"));
}

#[test]
fn policy_keygen_rejects_identical_paths_and_directories() {
    let temp = tempfile::tempdir().unwrap();
    let same = temp.path().join("same.json");
    let identical = keygen(&same, &same, true);
    assert!(!identical.status.success());
    assert!(String::from_utf8_lossy(&identical.stderr).contains("must be different"));

    let private = temp.path().join("private.json");
    let trust_dir = temp.path().join("trust-dir");
    fs::create_dir(&trust_dir).unwrap();
    let directory = keygen(&private, &trust_dir, true);
    assert!(!directory.status.success());
    assert!(String::from_utf8_lossy(&directory.stderr).contains("non-regular"));
    assert!(!private.exists());
}

#[test]
fn policy_keygen_force_rejects_symlinks() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().unwrap();
    let target = temp.path().join("target.json");
    let private = temp.path().join("private.json");
    let trust = temp.path().join("trust.json");
    fs::write(&target, "do-not-replace").unwrap();
    symlink(&target, &private).unwrap();
    let output = keygen(&private, &trust, true);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("non-regular"));
    assert_eq!(fs::read_to_string(target).unwrap(), "do-not-replace");
    assert!(!trust.exists());
}
