#!/usr/bin/env python3
"""Build, verify, sign, and optionally publish immutable Costguard release assets."""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))

from costguard_tooling import file_sha256, git_output, workspace_version  # noqa: E402
from release_packaging import (  # noqa: E402
    RELEASE_TARGETS,
    asset_name,
    ensure_targets_installed,
    package_and_verify_target,
    write_consolidated_checksums,
    write_provenance,
)


def require_command(name: str) -> None:
    if shutil.which(name) is None:
        raise SystemExit(f"required command not found: {name}")


def git_config(name: str) -> str:
    return subprocess.run(
        ["git", "config", "--get", name],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=False,
    ).stdout.strip()


def require_clean_worktree() -> None:
    if git_output("status", "--porcelain"):
        raise SystemExit("release publication requires a clean worktree")


def require_signed_tag(tag: str) -> tuple[str, int]:
    tag_type = git_output("cat-file", "-t", f"refs/tags/{tag}")
    if tag_type != "tag":
        raise SystemExit(f"release tag {tag} must be annotated")
    verify = subprocess.run(
        ["git", "tag", "-v", tag], cwd=ROOT, capture_output=True, text=True, check=False
    )
    if verify.returncode != 0:
        raise SystemExit(f"release tag signature verification failed:\n{verify.stderr}")
    commit = git_output("rev-list", "-n", "1", tag)
    head = git_output("rev-parse", "HEAD")
    if commit != head:
        raise SystemExit(f"release tag {tag} points to {commit}, not HEAD {head}")
    epoch = int(git_output("show", "-s", "--format=%ct", commit))
    return commit, epoch


def package_all_targets(root: Path, workdir: Path, *, epoch: int | None = None) -> list[Path]:
    workdir.mkdir(parents=True, exist_ok=True)
    assets: list[Path] = []
    for target in RELEASE_TARGETS:
        asset, checksum = package_and_verify_target(root, workdir, target, epoch=epoch)
        assets.extend([asset, checksum])
        print(f"packaged {asset.name}")
    return assets


def create_smoke_receipt(workdir: Path, target: str, version: str) -> Path:
    receipt = workdir / f"smoke-{target}.json"
    subprocess.run(
        [
            sys.executable,
            str(ROOT / "scripts/smoke_release_asset.py"),
            "--asset",
            str(workdir / asset_name(target)),
            "--target",
            target,
            "--version",
            version,
            "--receipt",
            str(receipt),
        ],
        cwd=ROOT,
        check=True,
    )
    return receipt


def verify_receipt(receipt: Path, workdir: Path, target: str, version: str) -> Path:
    payload = json.loads(receipt.read_text(encoding="utf-8"))
    asset = workdir / asset_name(target)
    expected = {
        "target": target,
        "version": version,
        "asset": asset.name,
        "sha256": file_sha256(asset),
    }
    for key, value in expected.items():
        if payload.get(key) != value:
            raise SystemExit(f"invalid smoke receipt {receipt}: {key} mismatch")
    if payload.get("schema_version") != 1:
        raise SystemExit(f"invalid smoke receipt {receipt}: schema_version mismatch")
    if payload.get("checks") != ["version", "rules-json"]:
        raise SystemExit(f"invalid smoke receipt {receipt}: checks mismatch")
    verified_on = str(payload.get("verified_on", ""))
    method = payload.get("verification_method")
    if target == "x86_64-pc-windows-msvc":
        valid_platform = verified_on.startswith("Windows-") and method == "native"
    elif target == "aarch64-apple-darwin":
        valid_platform = verified_on.startswith("Darwin-") and method == "native"
    elif target == "x86_64-apple-darwin":
        valid_platform = verified_on.startswith("Darwin-") and method in {"native", "rosetta"}
    else:
        valid_platform = (verified_on.startswith("Linux-") and method == "native") or (
            verified_on.startswith("Darwin-") and method == "docker"
        )
    if not valid_platform:
        raise SystemExit(f"invalid smoke receipt {receipt}: verification platform mismatch")
    destination = workdir / f"smoke-{target}.json"
    if receipt.resolve() != destination.resolve():
        shutil.copy2(receipt, destination)
    return destination


def collect_receipts(workdir: Path, version: str, supplied: list[Path]) -> list[Path]:
    by_target: dict[str, Path] = {}
    for receipt in supplied:
        payload = json.loads(receipt.read_text(encoding="utf-8"))
        target = payload.get("target")
        if target not in RELEASE_TARGETS:
            raise SystemExit(f"unknown receipt target in {receipt}: {target}")
        by_target[target] = verify_receipt(receipt, workdir, target, version)
    for target in RELEASE_TARGETS:
        if target not in by_target:
            if target == "x86_64-pc-windows-msvc":
                raise SystemExit("a native Windows smoke receipt is required for publication")
            by_target[target] = create_smoke_receipt(workdir, target, version)
    return [by_target[target] for target in RELEASE_TARGETS]


def prepare_package_receipts(workdir: Path, version: str, supplied: list[Path]) -> list[Path]:
    receipts: dict[str, Path] = {}
    for receipt in supplied:
        target = json.loads(receipt.read_text(encoding="utf-8")).get("target")
        if target not in RELEASE_TARGETS:
            raise SystemExit(f"unknown receipt target in {receipt}: {target}")
        receipts[target] = verify_receipt(receipt, workdir, target, version)
    for target in RELEASE_TARGETS:
        if target != "x86_64-pc-windows-msvc" and target not in receipts:
            receipts[target] = create_smoke_receipt(workdir, target, version)
    return [receipts[target] for target in RELEASE_TARGETS if target in receipts]


def verify_qualification_receipt(
    receipt: Path, workdir: Path, *, version: str, tag: str, commit: str
) -> Path:
    payload = json.loads(receipt.read_text(encoding="utf-8"))
    expected = {
        "schema_version": 1,
        "status": "passed",
        "version": version,
        "tag": tag,
        "commit": commit,
    }
    for key, value in expected.items():
        if payload.get(key) != value:
            raise SystemExit(f"invalid qualification receipt {receipt}: {key} mismatch")
    if not payload.get("commands"):
        raise SystemExit(f"invalid qualification receipt {receipt}: commands are missing")
    destination = workdir / "release-check.json"
    destination.parent.mkdir(parents=True, exist_ok=True)
    if receipt.resolve() != destination.resolve():
        shutil.copy2(receipt, destination)
    return destination


def sign_provenance(path: Path) -> Path:
    signing_key = git_config("user.signingkey")
    if not signing_key:
        raise SystemExit("user.signingkey must be configured to sign release provenance")
    signing_format = git_config("gpg.format") or "openpgp"
    if signing_format == "ssh":
        key_path = Path(signing_key).expanduser()
        if not key_path.exists():
            raise SystemExit("SSH release signing requires user.signingkey to be a private key path")
        require_command("ssh-keygen")
        subprocess.run(
            ["ssh-keygen", "-Y", "sign", "-f", str(key_path), "-n", "costguard-release", str(path)],
            check=True,
        )
        return Path(f"{path}.sig")
    require_command("gpg")
    signature = path.with_suffix(path.suffix + ".asc")
    subprocess.run(
        [
            "gpg",
            "--batch",
            "--yes",
            "--armor",
            "--local-user",
            signing_key,
            "--output",
            str(signature),
            "--detach-sign",
            str(path),
        ],
        check=True,
    )
    return signature


def release_exists(tag: str) -> bool:
    return subprocess.run(
        ["gh", "release", "view", tag], capture_output=True, text=True, check=False
    ).returncode == 0


def require_public_repository() -> None:
    visibility = subprocess.run(
        ["gh", "repo", "view", "hypertrial/costguard", "--json", "visibility", "--jq", ".visibility"],
        capture_output=True,
        text=True,
        check=True,
    ).stdout.strip()
    if visibility != "PUBLIC":
        raise SystemExit(f"release publication requires a PUBLIC repository, got {visibility}")


def publish_release(tag: str, files: list[Path], *, notes_file: Path | None) -> None:
    require_command("gh")
    require_public_repository()
    if release_exists(tag):
        raise SystemExit(f"release {tag} already exists; exact releases are immutable")
    command = [
        "gh",
        "release",
        "create",
        tag,
        *[str(path) for path in files],
        "--verify-tag",
        "--title",
        f"Costguard {tag}",
    ]
    if notes_file:
        command.extend(["--notes-file", str(notes_file)])
    else:
        command.append("--generate-notes")
    subprocess.run(command, cwd=ROOT, check=True)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--version", required=True)
    parser.add_argument("--workdir", type=Path, default=Path("dist/release"))
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--package-only", action="store_true")
    mode.add_argument("--publish", action="store_true")
    parser.add_argument("--receipt", action="append", type=Path, default=[])
    parser.add_argument("--qualification-receipt", type=Path)
    parser.add_argument("--notes-file", type=Path)
    args = parser.parse_args()

    version = args.version.removeprefix("v")
    tag = f"v{version}"
    if version != workspace_version(ROOT):
        raise SystemExit(f"release version {version} != workspace version {workspace_version(ROOT)}")
    for command in ["cargo", "rustup"]:
        require_command(command)
    require_clean_worktree()
    commit, epoch = require_signed_tag(tag)
    qualification_path = args.qualification_receipt or args.workdir / "release-check.json"
    qualification = verify_qualification_receipt(
        qualification_path, args.workdir, version=version, tag=tag, commit=commit
    )
    if args.publish:
        require_command("gh")
        require_public_repository()
        if release_exists(tag):
            raise SystemExit(f"release {tag} already exists; exact releases are immutable")
    ensure_targets_installed(RELEASE_TARGETS)
    os.environ["SOURCE_DATE_EPOCH"] = str(epoch)

    packaged = package_all_targets(ROOT, args.workdir, epoch=epoch)
    binary_assets = [path for path in packaged if path.name.endswith(".tar.gz")]
    checksums = write_consolidated_checksums(args.workdir, binary_assets)
    if args.package_only:
        receipts = prepare_package_receipts(args.workdir, version, args.receipt)
        print(
            f"prepared {len(binary_assets)} assets and {len(receipts)} native receipts for {tag}; "
            "run the Windows smoke test before publication"
        )
        return 0
    receipts = collect_receipts(args.workdir, version, args.receipt)
    provenance = write_provenance(
        ROOT,
        args.workdir,
        version=version,
        assets=binary_assets,
        receipts=[qualification, *receipts],
    )
    signature = sign_provenance(provenance)
    release_files = sorted(
        [*packaged, checksums, qualification, *receipts, provenance, signature]
    )
    expected_names = {
        *(asset_name(target) for target in RELEASE_TARGETS),
        *(f"{asset_name(target)}.sha256" for target in RELEASE_TARGETS),
        *(f"smoke-{target}.json" for target in RELEASE_TARGETS),
        "SHA256SUMS",
        "release-check.json",
        "provenance.json",
        signature.name,
    }
    actual_names = {path.name for path in release_files}
    if actual_names != expected_names or len(actual_names) != len(release_files):
        raise SystemExit(
            f"release asset inventory mismatch: expected {sorted(expected_names)}, "
            f"got {sorted(actual_names)}"
        )
    publish_release(tag, release_files, notes_file=args.notes_file)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
