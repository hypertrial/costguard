#!/usr/bin/env python3
"""Verify the exact GitHub release asset inventory and consolidated checksums."""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))

from costguard_tooling import file_sha256  # noqa: E402
from release_packaging import RELEASE_TARGETS, asset_name  # noqa: E402


def verify(workdir: Path, version: str) -> Path:
    expected = {
        *(asset_name(target) for target in RELEASE_TARGETS),
        *(f"{asset_name(target)}.sha256" for target in RELEASE_TARGETS),
        *(f"smoke-{target}.json" for target in RELEASE_TARGETS),
        "costguard.cdx.json",
        "release-check.json",
    }
    actual = {path.name for path in workdir.iterdir() if path.is_file()}
    allowed_existing = expected | {"SHA256SUMS"}
    if actual - allowed_existing or expected - actual:
        raise SystemExit(
            f"release asset inventory mismatch: expected {sorted(expected)}, got {sorted(actual)}"
        )
    archives = [workdir / asset_name(target) for target in RELEASE_TARGETS]
    for target, archive in zip(RELEASE_TARGETS, archives, strict=True):
        checksum = workdir / f"{archive.name}.sha256"
        fields = checksum.read_text(encoding="utf-8").split()
        if fields != [file_sha256(archive), archive.name]:
            raise SystemExit(f"invalid checksum file {checksum}")
        receipt = json.loads(
            (workdir / f"smoke-{target}.json").read_text(encoding="utf-8")
        )
        if receipt.get("target") != target or receipt.get("version") != version:
            raise SystemExit(f"invalid smoke receipt for {target}")
        if receipt.get("sha256") != file_sha256(archive):
            raise SystemExit(f"smoke receipt checksum mismatch for {target}")
    sums = workdir / "SHA256SUMS"
    sums.write_text(
        "".join(
            f"{file_sha256(path)}  {path.name}\n" for path in sorted(archives)
        ),
        encoding="utf-8",
    )
    return sums


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--workdir", type=Path, default=Path("dist/release"))
    parser.add_argument("--version", required=True)
    args = parser.parse_args()
    verify(args.workdir, args.version.removeprefix("v"))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
