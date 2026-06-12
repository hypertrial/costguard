#!/usr/bin/env python3
"""Enforce the Costguard 10k-model release performance budget."""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))

from costguard_tooling import costguard_binary  # noqa: E402


def max_rss_bytes(raw: int) -> int:
    # macOS reports bytes; Linux reports KiB.
    return raw if raw > 10_000_000 else raw * 1024


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--models", type=int, default=10_000)
    parser.add_argument("--max-seconds", type=float, default=10.0)
    parser.add_argument("--max-rss-bytes", type=int, default=1024**3)
    args = parser.parse_args()

    binary = costguard_binary(release=True)

    with tempfile.TemporaryDirectory(prefix="costguard-scale-") as tmp:
        project = Path(tmp)
        subprocess.run(
            [
                "python3",
                str(ROOT / "scripts/generate_synthetic_dbt.py"),
                str(project),
                "--models",
                str(args.models),
                "--clean",
            ],
            check=True,
        )
        started = time.monotonic()
        stdout_path = project / "scan.stdout"
        stderr_path = project / "scan.stderr"
        with stdout_path.open("wb") as stdout, stderr_path.open("wb") as stderr:
            process = subprocess.Popen(
                [
                    str(binary),
                    "scan",
                    "models",
                    "--manifest",
                    "target/manifest.json",
                    "--warehouse",
                    "generic",
                    "--fail-on",
                    "critical",
                    "--format",
                    "json",
                ],
                cwd=project,
                stdout=stdout,
                stderr=stderr,
            )
            _, status, usage = os.wait4(process.pid, 0)
            process.returncode = os.waitstatus_to_exitcode(status)
        elapsed = time.monotonic() - started
        returncode = process.returncode
        stdout_text = stdout_path.read_text(encoding="utf-8")
        stderr_text = stderr_path.read_text(encoding="utf-8")
        rss = max_rss_bytes(usage.ru_maxrss)

    if returncode != 0:
        raise SystemExit(f"scale scan failed:\n{stderr_text or stdout_text}")
    payload = json.loads(stdout_text)
    metrics = payload["metrics"]
    if metrics["sql_parse_total"] != args.models:
        raise SystemExit(
            f"expected {args.models} parsed SQL models, got {metrics['sql_parse_total']}"
        )
    if metrics["sql_parse_failures"] != 0:
        raise SystemExit(f"scale fixture had {metrics['sql_parse_failures']} parse failures")
    if payload["diagnostics"]:
        raise SystemExit(f"clean scale fixture emitted {len(payload['diagnostics'])} diagnostics")
    if elapsed > args.max_seconds:
        raise SystemExit(f"scale runtime {elapsed:.2f}s exceeded {args.max_seconds:.2f}s")
    if rss > args.max_rss_bytes:
        raise SystemExit(f"scale max RSS {rss} exceeded {args.max_rss_bytes}")
    print(f"scale gate passed: {args.models} models in {elapsed:.2f}s, max RSS {rss} bytes")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
