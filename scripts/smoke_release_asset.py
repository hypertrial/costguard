#!/usr/bin/env python3
"""Native smoke-test a release asset and emit a checksum-bound receipt."""

from __future__ import annotations

import argparse
import json
import platform
import subprocess
import tarfile
import tempfile
from pathlib import Path

from release_packaging import file_sha256, target_bin_name, verify_archive_layout


def command_prefix(binary: Path, target: str, extract_dir: Path) -> tuple[list[str], str]:
    system = platform.system()
    machine = platform.machine().lower()
    if target == "aarch64-apple-darwin" and system == "Darwin" and machine == "arm64":
        return [str(binary)], "native"
    if target == "x86_64-apple-darwin" and system == "Darwin":
        if machine in {"x86_64", "amd64"}:
            return [str(binary)], "native"
        return ["arch", "-x86_64", str(binary)], "rosetta"
    if target == "x86_64-unknown-linux-gnu" and system == "Linux":
        return [str(binary)], "native"
    if target == "x86_64-unknown-linux-gnu" and system == "Darwin":
        probe = subprocess.run(["docker", "info"], capture_output=True, check=False)
        if probe.returncode != 0:
            raise SystemExit("Docker is required to smoke-test the Linux asset on macOS")
        return (
            [
                "docker",
                "run",
                "--rm",
                "-v",
                f"{extract_dir}:/work:ro",
                "debian:bookworm-slim",
                "/work/costguard",
            ],
            "docker",
        )
    if target == "x86_64-pc-windows-msvc" and system == "Windows":
        return [str(binary)], "native"
    raise SystemExit(f"target {target} cannot be natively smoke-tested on {system}-{machine}")


def smoke_asset(
    asset: Path,
    target: str,
    *,
    expected_version: str | None = None,
    receipt: Path | None = None,
) -> str:
    """Extract a release archive and run version/rules smoke checks."""
    if receipt is not None and expected_version is None:
        raise SystemExit("receipt requires expected_version")
    bin_name = target_bin_name(target)
    verify_archive_layout(asset, bin_name=bin_name)
    with tempfile.TemporaryDirectory(prefix="costguard-smoke-") as tmp:
        extract_dir = Path(tmp)
        with tarfile.open(asset, "r:gz") as archive:
            archive.extractall(extract_dir, filter="data")
        binary = extract_dir / bin_name
        prefix, verification_method = command_prefix(binary, target, extract_dir)
        if expected_version is not None:
            version = subprocess.run(
                [*prefix, "--version"], capture_output=True, text=True, check=True
            ).stdout.strip()
            normalized = expected_version.removeprefix("v")
            if version != f"costguard {normalized}":
                raise SystemExit(f"unexpected version output: {version}")
        rules = subprocess.run(
            [*prefix, "rules", "--format", "json"],
            capture_output=True,
            text=True,
            check=True,
        )
        if not json.loads(rules.stdout):
            raise SystemExit("rules output was empty")
    if receipt is not None:
        normalized = expected_version.removeprefix("v")  # type: ignore[union-attr]
        receipt.parent.mkdir(parents=True, exist_ok=True)
        receipt.write_text(
            json.dumps(
                {
                    "schema_version": 1,
                    "target": target,
                    "version": normalized,
                    "asset": asset.name,
                    "sha256": file_sha256(asset),
                    "verified_on": f"{platform.system()}-{platform.machine()}",
                    "verification_method": verification_method,
                    "checks": ["version", "rules-json"],
                },
                indent=2,
                sort_keys=True,
            )
            + "\n",
            encoding="utf-8",
        )
        print(f"wrote smoke receipt {receipt}")
    return verification_method


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--asset", type=Path, required=True)
    parser.add_argument("--target", choices=list_targets(), required=True)
    parser.add_argument("--version", required=True)
    parser.add_argument("--receipt", type=Path, required=True)
    args = parser.parse_args()
    smoke_asset(
        args.asset,
        args.target,
        expected_version=args.version,
        receipt=args.receipt,
    )
    return 0


def list_targets() -> list[str]:
    from release_packaging import RELEASE_TARGETS

    return list(RELEASE_TARGETS)


if __name__ == "__main__":
    raise SystemExit(main())
