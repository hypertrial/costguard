#!/usr/bin/env python3
"""Enforce the Costguard 10k-model release performance budget."""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))

from costguard_tooling import (  # noqa: E402
    measure_costguard_scan,
    summarize_measurements,
)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--models", type=int, default=10_000)
    parser.add_argument("--max-median-seconds", type=float, default=10.0)
    parser.add_argument("--max-seconds", type=float, default=15.0)
    parser.add_argument("--max-rss-bytes", type=int, default=1024**3)
    parser.add_argument("--measurements", type=int, default=3)
    parser.add_argument(
        "--report",
        type=Path,
        default=ROOT / "tests/benchmarks/reports/scale.json",
    )
    args = parser.parse_args()
    if args.measurements < 1:
        raise SystemExit("measurements must be at least one")

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
        scan_args = {
            "workdir": project,
            "warehouse": "generic",
            "scan_paths": ["models"],
            "fail_on": "critical",
            "manifest": Path("target/manifest.json"),
        }
        measure_costguard_scan(**scan_args)
        measurements = [
            measure_costguard_scan(**scan_args) for _ in range(args.measurements)
        ]

    payload = measurements[0]["payload"]
    for measurement in measurements:
        if measurement["exit_code"] != 0:
            raise SystemExit("scale scan returned a failing exit code")
        if measurement["payload"]["metrics"] != payload["metrics"]:
            raise SystemExit("scale scan metrics changed between measured runs")
        if measurement["payload"].get("diagnostics") != payload.get("diagnostics"):
            raise SystemExit("scale scan diagnostics changed between measured runs")
    metrics = payload["metrics"]
    if metrics["sql_parse_total"] != args.models:
        raise SystemExit(
            f"expected {args.models} parsed SQL models, got {metrics['sql_parse_total']}"
        )
    if metrics["sql_parse_failures"] != 0:
        raise SystemExit(f"scale fixture had {metrics['sql_parse_failures']} parse failures")
    if payload["diagnostics"]:
        raise SystemExit(f"clean scale fixture emitted {len(payload['diagnostics'])} diagnostics")
    summary = summarize_measurements(measurements)
    median_limit_ms = int(args.max_median_seconds * 1000)
    max_limit_ms = int(args.max_seconds * 1000)
    if summary["runtime_median_ms"] > median_limit_ms:
        raise SystemExit(
            f"scale median runtime {summary['runtime_median_ms']}ms exceeded {median_limit_ms}ms"
        )
    if summary["runtime_max_ms"] > max_limit_ms:
        raise SystemExit(
            f"scale max runtime {summary['runtime_max_ms']}ms exceeded {max_limit_ms}ms"
        )
    if summary["max_rss_bytes"] > args.max_rss_bytes:
        raise SystemExit(
            f"scale max RSS {summary['max_rss_bytes']} exceeded {args.max_rss_bytes}"
        )
    report = {
        "version": 2,
        "target": "synthetic/10k" if args.models == 10_000 else f"synthetic/{args.models}",
        "models": args.models,
        **summary,
        "metrics": metrics,
        "diagnostics_count": len(payload.get("diagnostics", [])),
        "thresholds": {
            "max_runtime_median_ms": median_limit_ms,
            "max_runtime_max_ms": max_limit_ms,
            "max_rss_bytes": args.max_rss_bytes,
        },
    }
    args.report.parent.mkdir(parents=True, exist_ok=True)
    args.report.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(
        "scale gate passed: "
        f"{args.models} models, median {summary['runtime_median_ms']}ms, "
        f"max {summary['runtime_max_ms']}ms, max RSS {summary['max_rss_bytes']} bytes"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
