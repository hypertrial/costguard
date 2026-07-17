#!/usr/bin/env python3
"""Shared helpers for Costguard scripts and the GitHub Action driver."""

from __future__ import annotations

import hashlib
import json
import os
import platform
import subprocess
import tempfile
import time
import tomllib
from pathlib import Path
from statistics import median
from typing import Any

ROOT = Path(__file__).resolve().parents[1]
CRATES = ROOT / "crates"
REPOS_TOML = ROOT / "tests" / "benchmarks" / "repos.toml"
BENCHMARK_COST_CONFIGS = REPOS_TOML.parent / "cost-configs"
RELEASE_BIN_NAME = "costguard"
WINDOWS_BIN_NAME = "costguard.exe"


def max_rss_bytes(raw: int) -> int:
    """Normalize wait4 ru_maxrss to bytes on macOS and Linux."""
    return raw if raw > 10_000_000 else raw * 1024


def build_profile(*, release: bool | None = None) -> str:
    if release is not None:
        return "release" if release else "debug"
    env = os.environ.get("COSTGUARD_BUILD_PROFILE", "release").strip().lower()
    return "release" if env == "release" else "debug"


def newest_rust_source_mtime() -> float:
    newest = 0.0
    for path in CRATES.rglob("*.rs"):
        newest = max(newest, path.stat().st_mtime)
    for name in ("Cargo.toml", "Cargo.lock"):
        candidate = ROOT / name
        if candidate.exists():
            newest = max(newest, candidate.stat().st_mtime)
    return newest


def costguard_binary(*, release: bool | None = None) -> Path:
    profile = build_profile(release=release)
    target_dir = Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target"))
    binary = target_dir / profile / "costguard"
    needs_build = not binary.exists()
    if binary.exists():
        needs_build = binary.stat().st_mtime < newest_rust_source_mtime()
    if needs_build:
        cmd = ["cargo", "build", "-q", "-p", "costguard-cli"]
        if profile == "release":
            cmd.append("--release")
        build = subprocess.run(
            cmd,
            cwd=ROOT,
            capture_output=True,
            text=True,
            check=False,
        )
        if build.returncode != 0:
            raise SystemExit(f"failed to build costguard-cli:\n{build.stderr}")
    if not binary.exists():
        raise SystemExit(f"costguard binary not found at {binary}")
    return binary


def workspace_version(root: Path | None = None) -> str:
    repo_root = root or ROOT
    data = tomllib.loads((repo_root / "Cargo.toml").read_text(encoding="utf-8"))
    return data["workspace"]["package"]["version"]


def release_tag_version(root: Path | None = None) -> str:
    return f"v{workspace_version(root)}"


def git_output(*args: str, root: Path | None = None) -> str:
    repo_root = root or ROOT
    return subprocess.run(
        ["git", *args],
        cwd=repo_root,
        capture_output=True,
        text=True,
        check=True,
    ).stdout.strip()


def file_sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def host_target() -> tuple[str, str]:
    system = platform.system()
    machine = platform.machine().lower()
    if system == "Linux" and machine in {"x86_64", "amd64"}:
        return "x86_64-unknown-linux-gnu", RELEASE_BIN_NAME
    if system == "Darwin" and machine == "arm64":
        return "aarch64-apple-darwin", RELEASE_BIN_NAME
    if system == "Darwin" and machine == "x86_64":
        return "x86_64-apple-darwin", RELEASE_BIN_NAME
    if system in {"Windows", "MINGW", "MSYS", "CYGWIN"} and machine in {
        "x86_64",
        "amd64",
    }:
        return "x86_64-pc-windows-msvc", WINDOWS_BIN_NAME
    raise SystemExit(f"unsupported host platform: {system}-{machine}")


def load_repos(repos_toml: Path | None = None) -> list[dict[str, Any]]:
    path = repos_toml or REPOS_TOML
    data = tomllib.loads(path.read_text(encoding="utf-8"))
    return data.get("repo", [])


def repo_by_name(name: str, repos_toml: Path | None = None) -> dict[str, Any]:
    path = repos_toml or REPOS_TOML
    for repo in load_repos(path):
        if repo["name"] == name:
            return repo
    raise SystemExit(f"unknown repo '{name}' in {path}")


def apply_benchmark_cost_config(checkout: Path, repo: dict[str, Any]) -> bool:
    """Copy committed benchmark cost config into checkout if one exists."""
    src = BENCHMARK_COST_CONFIGS / f"{repo['name']}.toml"
    if not src.is_file():
        return False
    (checkout / "costguard.toml").write_text(src.read_text(encoding="utf-8"), encoding="utf-8")
    return True


def run_costguard_scan(
    workdir: Path,
    *,
    warehouse: str,
    scan_paths: list[str],
    fail_on: str = "critical",
    manifest: Path | None = None,
    cost: bool = False,
) -> dict[str, Any]:
    cmd = [
        str(costguard_binary()),
        "scan",
        "--warehouse",
        warehouse,
        "--fail-on",
        fail_on,
        "--format",
        "json",
    ]
    if cost:
        cmd.append("--cost")
    if manifest is not None:
        if manifest.is_absolute():
            manifest_arg = (
                manifest.relative_to(workdir) if manifest.is_relative_to(workdir) else manifest
            )
        else:
            manifest_arg = manifest
        cmd.extend(["--manifest", str(manifest_arg)])
    cmd.extend(scan_paths)

    completed = subprocess.run(
        cmd,
        cwd=workdir,
        capture_output=True,
        text=True,
        check=False,
    )
    if completed.returncode not in (0, 1):
        raise SystemExit(
            f"costguard scan failed (exit {completed.returncode}):\n{completed.stderr.strip()}"
        )

    try:
        payload = json.loads(completed.stdout)
    except json.JSONDecodeError as exc:
        raise SystemExit(
            f"failed to parse costguard JSON output: {exc}\nstdout:\n{completed.stdout}"
        ) from exc

    if "metrics" not in payload:
        raise SystemExit("costguard JSON output missing 'metrics'")

    return payload, completed.returncode


def measure_costguard_scan(
    workdir: Path,
    *,
    warehouse: str,
    scan_paths: list[str],
    fail_on: str = "critical",
    manifest: Path | None = None,
    cost: bool = False,
) -> dict[str, Any]:
    """Run one scan and return its payload plus wall-clock and peak-RSS data."""
    command = [
        "scan",
        "--warehouse",
        warehouse,
        "--fail-on",
        fail_on,
        "--format",
        "json",
    ]
    if cost:
        command.append("--cost")
    if manifest is not None:
        if manifest.is_absolute():
            manifest_arg = (
                manifest.relative_to(workdir) if manifest.is_relative_to(workdir) else manifest
            )
        else:
            manifest_arg = manifest
        command.extend(["--manifest", str(manifest_arg)])
    command.extend(scan_paths)
    return measure_costguard_command(workdir, command)


def measure_costguard_pr(
    workdir: Path,
    *,
    base: str,
    warehouse: str,
    manifest: Path,
) -> dict[str, Any]:
    """Run one PR replay and return its payload plus wall-clock and peak-RSS data."""
    return measure_costguard_command(
        workdir,
        [
            "pr",
            "--base",
            base,
            "--warehouse",
            warehouse,
            "--fail-on",
            "critical",
            "--format",
            "json",
            "--manifest",
            str(manifest),
        ],
    )


def measure_costguard_command(workdir: Path, command: list[str]) -> dict[str, Any]:
    cmd = [str(costguard_binary()), *command]

    with tempfile.TemporaryDirectory(prefix="costguard-measure-") as tmp:
        output = Path(tmp) / "stdout.json"
        errors = Path(tmp) / "stderr.txt"
        started = time.monotonic()
        with output.open("wb") as stdout, errors.open("wb") as stderr:
            process = subprocess.Popen(cmd, cwd=workdir, stdout=stdout, stderr=stderr)
            _, status, usage = os.wait4(process.pid, 0)
            process.returncode = os.waitstatus_to_exitcode(status)
        runtime_ms = int((time.monotonic() - started) * 1000)
        stdout_text = output.read_text(encoding="utf-8")
        stderr_text = errors.read_text(encoding="utf-8")

    if process.returncode not in (0, 1):
        raise SystemExit(
            f"costguard scan failed (exit {process.returncode}):\n{stderr_text.strip()}"
        )
    try:
        payload = json.loads(stdout_text)
    except json.JSONDecodeError as exc:
        raise SystemExit(
            f"failed to parse costguard JSON output: {exc}\nstdout:\n{stdout_text}"
        ) from exc
    if "metrics" not in payload:
        raise SystemExit("costguard JSON output missing 'metrics'")
    return {
        "payload": payload,
        "exit_code": process.returncode,
        "runtime_ms": runtime_ms,
        "max_rss_bytes": max_rss_bytes(usage.ru_maxrss),
    }


def summarize_measurements(measurements: list[dict[str, Any]]) -> dict[str, Any]:
    if not measurements:
        raise ValueError("at least one measurement is required")
    runtime_samples = [int(item["runtime_ms"]) for item in measurements]
    return {
        "runtime_samples_ms": runtime_samples,
        "runtime_median_ms": int(median(runtime_samples)),
        "runtime_max_ms": max(runtime_samples),
        "max_rss_bytes": max(int(item["max_rss_bytes"]) for item in measurements),
    }
