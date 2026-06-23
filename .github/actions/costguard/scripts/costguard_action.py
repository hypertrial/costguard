#!/usr/bin/env python3
"""Runtime driver for the Costguard composite GitHub Action."""

from __future__ import annotations

import argparse
import os
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
import time
import urllib.request
from pathlib import Path
from urllib.error import URLError

DEFAULT_MANIFEST = "target/manifest.json"
PRODUCER_REPOSITORY = "hypertrial/costguard"
DOWNLOAD_ATTEMPTS = 3
DOWNLOAD_TIMEOUT_SECONDS = 30
RELEASE_ARCHIVE_MAX_BYTES = 67_108_864
CHECKSUM_MAX_BYTES = 4_096
DOWNLOAD_CHUNK_BYTES = 64 * 1024


class DownloadTooLarge(Exception):
    """Deterministic download-size failure that must not be retried."""


def env(name: str, default: str = "") -> str:
    return os.environ.get(name, default).strip()


def action_path() -> Path:
    value = env("GITHUB_ACTION_PATH")
    if value:
        return Path(value).resolve()
    return Path(__file__).resolve().parents[1]


def action_repo_root() -> Path:
    return action_path().parents[2]


def ensure_scripts_path() -> None:
    scripts = action_repo_root() / "scripts"
    if str(scripts) not in sys.path:
        sys.path.insert(0, str(scripts))


def action_release_version() -> str:
    ensure_scripts_path()
    from costguard_tooling import release_tag_version  # noqa: E402

    return release_tag_version(action_repo_root())


def append_file(path: Path, value: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("a", encoding="utf-8") as handle:
        handle.write(value + "\n")


def runner_target() -> tuple[str, str]:
    ensure_scripts_path()
    from costguard_tooling import host_target  # noqa: E402

    return host_target()


def download(url: str, destination: Path, max_bytes: int) -> None:
    last_error: Exception | None = None
    for attempt in range(1, DOWNLOAD_ATTEMPTS + 1):
        try:
            with urllib.request.urlopen(url, timeout=DOWNLOAD_TIMEOUT_SECONDS) as response:
                content_length = (
                    response.headers.get("Content-Length")
                    if hasattr(response, "headers")
                    else None
                )
                if content_length is not None:
                    try:
                        declared = int(content_length)
                    except ValueError as exc:
                        raise OSError(f"invalid Content-Length: {content_length}") from exc
                    if declared > max_bytes:
                        raise DownloadTooLarge(
                            f"download {url} declares {declared} bytes, "
                            f"exceeding limit of {max_bytes} bytes"
                        )
                observed = 0
                with destination.open("wb") as output:
                    while chunk := response.read(DOWNLOAD_CHUNK_BYTES):
                        observed += len(chunk)
                        if observed > max_bytes:
                            raise DownloadTooLarge(
                                f"download {url} exceeded limit of {max_bytes} bytes "
                                f"while streaming ({observed} bytes observed)"
                            )
                        output.write(chunk)
            return
        except DownloadTooLarge as exc:
            destination.unlink(missing_ok=True)
            raise SystemExit(str(exc)) from exc
        except (OSError, URLError) as exc:
            last_error = exc
            destination.unlink(missing_ok=True)
            if attempt < DOWNLOAD_ATTEMPTS:
                time.sleep(attempt)
    raise SystemExit(f"failed to download {url} after {DOWNLOAD_ATTEMPTS} attempts: {last_error}")


def sha256(path: Path) -> str:
    ensure_scripts_path()
    from costguard_tooling import file_sha256  # noqa: E402

    return file_sha256(path)


def install_release(version: str) -> None:
    target, bin_name = runner_target()
    asset_name = f"costguard-{target}.tar.gz"
    base_url = env(
        "COSTGUARD_RELEASE_BASE_URL",
        f"https://github.com/hypertrial/costguard/releases/download/{version}",
    ).rstrip("/")
    runner_temp = Path(env("RUNNER_TEMP", tempfile.gettempdir()))
    install_dir = runner_temp / "costguard-bin"
    install_dir.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="costguard-action-") as tmp:
        temp_dir = Path(tmp)
        asset = temp_dir / asset_name
        checksum = temp_dir / f"{asset_name}.sha256"
        download(f"{base_url}/{asset_name}", asset, RELEASE_ARCHIVE_MAX_BYTES)
        download(f"{base_url}/{asset_name}.sha256", checksum, CHECKSUM_MAX_BYTES)
        if env("VERIFY_ATTESTATION_INPUT", "true").lower() == "true":
            verify_attestation(asset)
        checksum_fields = checksum.read_text(encoding="utf-8").split()
        if (
            len(checksum_fields) != 2
            or checksum_fields[1] != asset_name
            or re.fullmatch(r"[0-9a-fA-F]{64}", checksum_fields[0]) is None
        ):
            raise SystemExit(f"invalid checksum file for {asset_name}")
        expected = checksum_fields[0]
        actual = sha256(asset)
        if actual != expected:
            raise SystemExit(
                f"checksum mismatch for {asset_name}: expected {expected}, got {actual}"
            )
        with tarfile.open(asset, "r:gz") as archive:
            members = archive.getmembers()
            names = [member.name for member in members]
            if names != [bin_name] or not members[0].isfile():
                raise SystemExit(f"unexpected archive layout: {names}")
            archive.extractall(install_dir, filter="data")
    if bin_name != "costguard.exe":
        (install_dir / bin_name).chmod(0o755)
    append_file(Path(env("GITHUB_PATH")), str(install_dir))


def verify_attestation(asset: Path) -> None:
    gh = shutil.which("gh")
    if gh is None:
        raise SystemExit("gh is required to verify release artifact attestations")
    completed = subprocess.run(
        [gh, "attestation", "verify", str(asset), "--repo", PRODUCER_REPOSITORY],
        capture_output=True,
        text=True,
        check=False,
    )
    if completed.returncode != 0:
        detail = completed.stderr.strip() or completed.stdout.strip()
        raise SystemExit(f"artifact attestation verification failed: {detail}")


def install_source() -> None:
    root = action_repo_root()
    subprocess.run(
        ["cargo", "build", "--release", "--locked", "-p", "costguard-cli"],
        cwd=root,
        check=True,
    )
    append_file(Path(env("GITHUB_PATH")), str(root / "target" / "release"))


def command_install(args: argparse.Namespace) -> int:
    if args.mode == "source":
        install_source()
        return 0
    if args.mode != "release":
        raise SystemExit(f"unknown install mode: {args.mode}")
    version = args.version or action_release_version()
    install_release(version)
    return 0


def consumer_root() -> Path:
    workspace = Path(env("GITHUB_WORKSPACE", str(Path.cwd()))).resolve()
    working_directory = env("WORKING_DIRECTORY_INPUT", ".") or "."
    root = (workspace / working_directory).resolve()
    if root != workspace and workspace not in root.parents:
        raise SystemExit(f"working-directory resolves outside GITHUB_WORKSPACE: {root}")
    return root


def resolve_manifest(root: Path) -> str:
    manifest = env("MANIFEST_INPUT")
    if manifest:
        return manifest
    default = root / DEFAULT_MANIFEST
    if default.is_file():
        return DEFAULT_MANIFEST
    return ""


def command_run() -> int:
    root = consumer_root()
    summary = env("GITHUB_STEP_SUMMARY")
    summary_file: Path | None = None
    if summary:
        runner_temp = Path(env("RUNNER_TEMP", tempfile.gettempdir()))
        runner_temp.mkdir(parents=True, exist_ok=True)
        handle, name = tempfile.mkstemp(prefix="costguard-summary-", suffix=".md", dir=runner_temp)
        os.close(handle)
        summary_file = Path(name)
    command = [
        "costguard",
        "pr",
        "--base",
        env("BASE_INPUT", "origin/main"),
        "--warehouse",
        env("WAREHOUSE_INPUT", "generic"),
        "--fail-on",
        env("FAIL_ON_INPUT", "high"),
        "--format",
        env("FORMAT_INPUT", "github"),
        "--analysis-policy",
        env("ANALYSIS_POLICY_INPUT", "standard"),
    ]
    block_only_new = env("BLOCK_ONLY_NEW_INPUT", "true").lower()
    if block_only_new not in {"true", "false"}:
        raise SystemExit("block-only-new must be true or false")
    command.append(f"--block-only-new={block_only_new}")
    if summary_file:
        command.extend(["--summary-file", str(summary_file)])
    min_confidence = env("MIN_CONFIDENCE_INPUT")
    if min_confidence:
        command.extend(["--min-confidence", min_confidence])
    baseline = env("BASELINE_INPUT")
    if baseline:
        command.extend(["--baseline", baseline])
    if env("COST_INPUT").lower() == "true":
        command.append("--cost")
    fail_on_cost_delta = env("FAIL_ON_COST_DELTA_INPUT")
    if fail_on_cost_delta:
        command.extend(["--fail-on-cost-delta", fail_on_cost_delta])
    fail_on_pr_cost_increase = env("FAIL_ON_PR_COST_INCREASE_INPUT")
    if fail_on_pr_cost_increase:
        command.extend(["--fail-on-pr-cost-increase", fail_on_pr_cost_increase])
    manifest = resolve_manifest(root)
    if manifest:
        manifest_path = (root / manifest).resolve()
        if not manifest_path.is_file():
            raise SystemExit(f"manifest does not exist: {manifest}")
        command.extend(["--manifest", manifest])
    optional_pairs = [
        ("POLICY_BUNDLE_INPUT", "--policy"),
        ("TRUST_STORE_INPUT", "--trust-store"),
        ("POLICY_ORGANIZATION_INPUT", "--policy-organization"),
        ("POLICY_TEAM_INPUT", "--policy-team"),
        ("POLICY_REPOSITORY_INPUT", "--policy-repository"),
        ("RECEIPT_PATH_INPUT", "--receipt-file"),
        ("COMPARE_RECEIPT_INPUT", "--compare-receipt"),
    ]
    for env_name, flag in optional_pairs:
        value = env(env_name)
        if value:
            command.extend([flag, value])
    completed = subprocess.run(command, cwd=root, capture_output=True, text=True, check=False)
    sys.stdout.write(completed.stdout)
    sys.stderr.write(completed.stderr)
    if summary_file:
        if summary_file.stat().st_size:
            append_file(Path(summary), summary_file.read_text(encoding="utf-8").rstrip("\n"))
        summary_file.unlink(missing_ok=True)
    return completed.returncode


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser(description=__doc__)
    commands = result.add_subparsers(dest="command", required=True)
    install = commands.add_parser("install")
    install.add_argument("--mode", required=True)
    install.add_argument("--version", default="")
    commands.add_parser("run")
    return result


def main() -> int:
    args = parser().parse_args()
    if args.command == "install":
        return command_install(args)
    if args.command == "run":
        return command_run()
    raise AssertionError(args.command)


if __name__ == "__main__":
    raise SystemExit(main())
