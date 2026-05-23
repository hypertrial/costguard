#!/usr/bin/env python3
"""Build, package, and verify release assets match the GitHub Action install contract."""

from __future__ import annotations

import argparse
import json
import os
import platform
import shutil
import subprocess
import sys
import tarfile
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))

from costguard_tooling import costguard_binary  # noqa: E402

RELEASE_BIN_NAME = "costguard"
WINDOWS_BIN_NAME = "costguard.exe"


def host_target() -> tuple[str, str]:
    system = platform.system()
    machine = platform.machine().lower()
    if system == "Linux" and machine in {"x86_64", "amd64"}:
        return "x86_64-unknown-linux-gnu", RELEASE_BIN_NAME
    if system == "Darwin" and machine == "arm64":
        return "aarch64-apple-darwin", RELEASE_BIN_NAME
    if system == "Darwin" and machine == "x86_64":
        return "x86_64-apple-darwin", RELEASE_BIN_NAME
    if system in {"Windows", "MINGW", "MSYS", "CYGWIN"} and machine in {"x86_64", "amd64"}:
        return "x86_64-pc-windows-msvc", WINDOWS_BIN_NAME
    raise SystemExit(f"unsupported host platform for release verification: {system}-{machine}")


def asset_name(target: str) -> str:
    return f"costguard-{target}.tar.gz"


def package_release_binary(workdir: Path, *, target: str, bin_name: str) -> tuple[Path, Path]:
    binary = costguard_binary(release=True)
    dist_dir = workdir / target
    dist_dir.mkdir(parents=True, exist_ok=True)
    packaged_bin = dist_dir / bin_name
    shutil.copy2(binary, packaged_bin)
    packaged_bin.chmod(0o755)

    asset = workdir / asset_name(target)
    with tarfile.open(asset, "w:gz") as archive:
        archive.add(packaged_bin, arcname=bin_name)

    checksum_file = workdir / f"{asset.name}.sha256"
    digest = subprocess.run(
        ["shasum", "-a", "256", asset.name],
        cwd=workdir,
        capture_output=True,
        text=True,
        check=False,
    )
    if digest.returncode != 0:
        raise SystemExit(f"failed to compute sha256:\n{digest.stderr}")
    checksum_file.write_text(digest.stdout, encoding="utf-8")
    return asset, checksum_file


def verify_checksum(workdir: Path, asset: Path, checksum_file: Path) -> None:
    verify = subprocess.run(
        ["shasum", "-a", "256", "-c", checksum_file.name],
        cwd=workdir,
        capture_output=True,
        text=True,
        check=False,
    )
    if verify.returncode != 0:
        raise SystemExit(
            "checksum verification failed:\n"
            f"{verify.stdout}\n{verify.stderr}"
        )


def extract_and_smoke_test(asset: Path, *, bin_name: str) -> None:
    with tempfile.TemporaryDirectory(prefix="costguard-release-") as tmp:
        extract_dir = Path(tmp)
        with tarfile.open(asset, "r:gz") as archive:
            archive.extractall(extract_dir)
        binary = extract_dir / bin_name
        if not binary.exists():
            raise SystemExit(f"expected binary {bin_name} at archive root")
        proc = subprocess.run(
            [str(binary), "rules", "--format", "json"],
            capture_output=True,
            text=True,
            check=False,
        )
        if proc.returncode != 0:
            raise SystemExit(
                "extracted binary failed rules smoke test:\n"
                f"{proc.stderr or proc.stdout}"
            )
        payload = json.loads(proc.stdout)
        if not isinstance(payload, list) or not payload:
            raise SystemExit("rules output was empty or invalid")


def verify_release_assets(*, workdir: Path | None = None) -> None:
    target, bin_name = host_target()
    cleanup = workdir is None
    if workdir is None:
        workdir = Path(tempfile.mkdtemp(prefix="costguard-release-verify-"))
    try:
        asset, checksum_file = package_release_binary(workdir, target=target, bin_name=bin_name)
        verify_checksum(workdir, asset, checksum_file)
        extract_and_smoke_test(asset, bin_name=bin_name)
        print(f"verified release asset {asset.name} for target {target}")
    finally:
        if cleanup:
            shutil.rmtree(workdir, ignore_errors=True)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--workdir",
        type=Path,
        default=None,
        help="Optional directory to leave packaged assets in for inspection",
    )
    args = parser.parse_args()
    verify_release_assets(workdir=args.workdir)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
