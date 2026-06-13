#!/usr/bin/env python3
"""Package and natively smoke-test one already-built release target."""

from __future__ import annotations

import argparse
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))

from release_packaging import RELEASE_TARGETS, package_and_verify_target  # noqa: E402


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", choices=RELEASE_TARGETS, required=True)
    parser.add_argument("--version", required=True)
    parser.add_argument("--workdir", type=Path, default=Path("dist/release"))
    args = parser.parse_args()
    asset, _checksum = package_and_verify_target(
        ROOT, args.workdir, args.target, build=False
    )
    receipt = args.workdir / f"smoke-{args.target}.json"
    subprocess.run(
        [
            sys.executable,
            str(ROOT / "scripts/smoke_release_asset.py"),
            "--asset",
            str(asset),
            "--target",
            args.target,
            "--version",
            args.version,
            "--receipt",
            str(receipt),
        ],
        cwd=ROOT,
        check=True,
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
