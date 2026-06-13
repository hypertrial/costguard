#!/usr/bin/env python3
"""Enforce Costguard's release performance and scaling budgets."""

from __future__ import annotations

import argparse
import json
import os
import platform
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))

from costguard_tooling import (  # noqa: E402
    git_output,
    measure_costguard_scan,
    summarize_measurements,
    workspace_version,
)


def environment_metadata() -> dict[str, object]:
    try:
        commit = git_output("rev-parse", "HEAD")
    except subprocess.CalledProcessError:
        commit = "unknown"
    return {
        "costguard_version": workspace_version(),
        "git_commit": commit,
        "os": platform.system(),
        "os_release": platform.release(),
        "machine": platform.machine(),
        "python_version": platform.python_version(),
        "logical_cpus": os.cpu_count(),
    }


def run_scale_target(models: int, measurements: int) -> dict[str, Any]:
    with tempfile.TemporaryDirectory(prefix=f"costguard-scale-{models}-") as tmp:
        project = Path(tmp)
        subprocess.run(
            [
                "python3",
                str(ROOT / "scripts/generate_synthetic_dbt.py"),
                str(project),
                "--models",
                str(models),
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
        samples = [measure_costguard_scan(**scan_args) for _ in range(measurements)]

    payload = samples[0]["payload"]
    violations: list[str] = []
    for sample in samples:
        if sample["exit_code"] != 0:
            violations.append(f"{models}-model scan returned exit code {sample['exit_code']}")
        if sample["payload"]["metrics"] != payload["metrics"]:
            violations.append(f"{models}-model scan metrics changed between measured runs")
        if sample["payload"].get("diagnostics") != payload.get("diagnostics"):
            violations.append(f"{models}-model diagnostics changed between measured runs")

    metrics = payload["metrics"]
    if metrics["sql_parse_total"] != models:
        violations.append(
            f"{models}-model scan parsed {metrics['sql_parse_total']} SQL models"
        )
    if metrics["sql_parse_failures"] != 0:
        violations.append(
            f"{models}-model scan had {metrics['sql_parse_failures']} parse failures"
        )
    diagnostics_count = len(payload.get("diagnostics", []))
    if diagnostics_count:
        violations.append(
            f"{models}-model clean fixture emitted {diagnostics_count} diagnostics"
        )

    return {
        "target": f"synthetic/{models}",
        "models": models,
        **summarize_measurements(samples),
        "metrics": metrics,
        "diagnostics_count": diagnostics_count,
        "violations": violations,
    }


def threshold_violations(
    baseline: dict[str, Any],
    release: dict[str, Any],
    *,
    max_median_ms: int,
    max_runtime_ms: int,
    max_rss_bytes: int,
    max_growth_ratio: float,
) -> tuple[list[str], float | None]:
    violations = [*baseline.get("violations", []), *release.get("violations", [])]
    if release["runtime_median_ms"] > max_median_ms:
        violations.append(
            f"scale median runtime {release['runtime_median_ms']}ms exceeded {max_median_ms}ms"
        )
    if release["runtime_max_ms"] > max_runtime_ms:
        violations.append(
            f"scale max runtime {release['runtime_max_ms']}ms exceeded {max_runtime_ms}ms"
        )
    peak_rss = max(baseline["max_rss_bytes"], release["max_rss_bytes"])
    if peak_rss > max_rss_bytes:
        violations.append(f"scale max RSS {peak_rss} exceeded {max_rss_bytes}")

    growth_ratio = None
    if baseline["runtime_median_ms"] <= 0:
        violations.append("baseline median runtime must be positive")
    else:
        baseline_per_model = baseline["runtime_median_ms"] / baseline["models"]
        release_per_model = release["runtime_median_ms"] / release["models"]
        growth_ratio = release_per_model / baseline_per_model
        if growth_ratio > max_growth_ratio:
            violations.append(
                "per-model runtime growth ratio "
                f"{growth_ratio:.3f} exceeded {max_growth_ratio:.3f}"
            )
    return violations, growth_ratio


def write_report(path: Path, report: dict[str, Any], violations: list[str]) -> int:
    report["status"] = "failed" if violations else "passed"
    report["violations"] = violations
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    return 1 if violations else 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline-models", type=int, default=2_000)
    parser.add_argument("--models", type=int, default=10_000)
    parser.add_argument("--max-median-seconds", type=float, default=10.0)
    parser.add_argument("--max-seconds", type=float, default=15.0)
    parser.add_argument("--max-rss-bytes", type=int, default=1024**3)
    parser.add_argument("--max-growth-ratio", type=float, default=1.5)
    parser.add_argument("--measurements", type=int, default=3)
    parser.add_argument(
        "--report",
        type=Path,
        default=ROOT / "tests/benchmarks/reports/scale.json",
    )
    args = parser.parse_args()
    thresholds = {
        "max_runtime_median_ms": int(args.max_median_seconds * 1000),
        "max_runtime_max_ms": int(args.max_seconds * 1000),
        "max_rss_bytes": args.max_rss_bytes,
        "max_per_model_growth_ratio": args.max_growth_ratio,
        "development_target_median_ms": 6_000,
    }
    report: dict[str, Any] = {
        "version": 3,
        "environment": environment_metadata(),
        "thresholds": thresholds,
        "targets": {},
        "growth": None,
        "max_rss_bytes": None,
    }

    violations = []
    if args.measurements < 1:
        violations.append("--measurements must be at least one")
    if args.baseline_models < 1 or args.models <= args.baseline_models:
        violations.append("--models must be greater than positive --baseline-models")
    if args.max_growth_ratio <= 0:
        violations.append("--max-growth-ratio must be positive")
    if violations:
        return write_report(args.report, report, violations)

    try:
        baseline = run_scale_target(args.baseline_models, args.measurements)
        report["targets"]["baseline"] = baseline
        release = run_scale_target(args.models, args.measurements)
        report["targets"]["release"] = release
        violations, growth_ratio = threshold_violations(
            baseline,
            release,
            max_median_ms=thresholds["max_runtime_median_ms"],
            max_runtime_ms=thresholds["max_runtime_max_ms"],
            max_rss_bytes=args.max_rss_bytes,
            max_growth_ratio=args.max_growth_ratio,
        )
        report["growth"] = {
            "baseline_ms_per_model": baseline["runtime_median_ms"]
            / baseline["models"],
            "release_ms_per_model": release["runtime_median_ms"] / release["models"],
            "per_model_runtime_ratio": growth_ratio,
        }
        report["max_rss_bytes"] = max(
            baseline["max_rss_bytes"], release["max_rss_bytes"]
        )
    except Exception as exc:
        violations = [f"scale execution failed: {exc}"]

    status = write_report(args.report, report, violations)
    if status:
        for violation in violations:
            print(f"FAIL {violation}", file=sys.stderr)
        return status

    release = report["targets"]["release"]
    print(
        "scale gate passed: "
        f"{release['models']} models, median {release['runtime_median_ms']}ms, "
        f"max {release['runtime_max_ms']}ms, max RSS {report['max_rss_bytes']} bytes, "
        f"growth ratio {report['growth']['per_model_runtime_ratio']:.3f}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
