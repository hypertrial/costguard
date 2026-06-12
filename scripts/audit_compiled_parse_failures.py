#!/usr/bin/env python3
"""Audit compiled SQL parse failures from a dbt manifest.json."""

from __future__ import annotations

import argparse
import os
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def audit_binary() -> Path:
    target_dir = Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target"))
    binary = target_dir / "debug" / "audit-compiled-parse"
    build = subprocess.run(
        ["cargo", "build", "-q", "-p", "costguard-sql", "--bin", "audit-compiled-parse", "--features", "audit-bin"],
        cwd=ROOT,
        check=False,
    )
    if build.returncode != 0:
        raise SystemExit("failed to build audit-compiled-parse")
    if not binary.exists():
        raise SystemExit(f"audit binary not found at {binary}")
    return binary


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("manifest", type=Path, help="Path to manifest.json")
    parser.add_argument("--bucket", action="store_true", help="Print error bucket summary")
    parser.add_argument("--model", help="Inspect a single model by name")
    parser.add_argument("--json", action="store_true", help="Emit JSON report")
    args = parser.parse_args()

    if not args.manifest.exists():
        raise SystemExit(f"manifest not found: {args.manifest}")

    cmd = [str(audit_binary()), str(args.manifest)]
    if args.bucket:
        cmd.insert(1, "--bucket")
    if args.model:
        cmd.extend(["--model", args.model])
    if args.json:
        cmd.insert(1, "--json")

    completed = subprocess.run(cmd, cwd=ROOT, check=False)
    raise SystemExit(completed.returncode)


if __name__ == "__main__":
    main()
