#!/usr/bin/env python3
"""Shared helpers for building and locating the costguard CLI."""

from __future__ import annotations

import os
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CRATES = ROOT / "crates"


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
