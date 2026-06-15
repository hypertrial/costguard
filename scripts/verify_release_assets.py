#!/usr/bin/env python3
"""Build, package, and verify release assets match the GitHub Action install contract."""

from __future__ import annotations

import argparse
import shutil
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))

from costguard_tooling import costguard_binary  # noqa: E402
from release_packaging import (  # noqa: E402
    host_target,
    package_built_binary,
    verify_checksum,
)
from smoke_release_asset import smoke_asset  # noqa: E402


def package_host_release_binary(workdir: Path, *, target: str) -> tuple[Path, Path]:
    binary = costguard_binary(release=True)
    return package_built_binary(workdir, target=target, binary_path=binary)


def verify_release_assets(*, workdir: Path | None = None) -> None:
    target, _ = host_target()
    cleanup = workdir is None
    if workdir is None:
        workdir = Path(tempfile.mkdtemp(prefix="costguard-release-verify-"))
    try:
        asset, checksum_file = package_host_release_binary(workdir, target=target)
        verify_checksum(workdir, asset, checksum_file)
        smoke_asset(asset, target)
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
