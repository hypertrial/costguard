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
    measure_costguard_pr,
    measure_costguard_scan,
    summarize_measurements,
    workspace_version,
)

PR_REPLAY_MEDIAN_MS = 30_000
PR_REPLAY_MAX_MS = 45_000


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


def git(project: Path, *args: str) -> None:
    subprocess.run(
        ["git", *args],
        cwd=project,
        check=True,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
        text=True,
    )


def commit_fixture(project: Path, message: str) -> None:
    git(project, "add", ".")
    git(
        project,
        "-c",
        "user.name=Costguard Scale",
        "-c",
        "user.email=scale@costguard.local",
        "commit",
        "-q",
        "-m",
        message,
    )


def update_replay_model(project: Path, sql: str) -> None:
    model = project / "models/generated/model_0000.sql"
    model.write_text(sql, encoding="utf-8")
    manifest_path = project / "target/manifest.json"
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    manifest["nodes"]["model.synthetic.model_0000"]["compiled_code"] = sql
    manifest_path.write_text(json.dumps(manifest, indent=2), encoding="utf-8")


def run_pr_replay_target(models: int, measurements: int) -> dict[str, Any]:
    with tempfile.TemporaryDirectory(prefix=f"costguard-pr-scale-{models}-") as tmp:
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
        (project / "dbt_project.yml").write_text(
            "name: synthetic\nversion: 1.0.0\nconfig-version: 2\n"
            'model-paths: ["models"]\n',
            encoding="utf-8",
        )
        update_replay_model(
            project,
            "select json_extract(payload, '$.a'), json_extract(payload, '$.a') "
            "from source_table\n",
        )
        git(project, "init", "-q")
        commit_fixture(project, "generated base project")
        update_replay_model(project, "select 0 as id\n")
        commit_fixture(project, "clean one model")

        replay_args = {
            "workdir": project,
            "base": "HEAD~1",
            "warehouse": "generic",
            "manifest": Path("target/manifest.json"),
        }
        measure_costguard_pr(**replay_args)
        samples = [measure_costguard_pr(**replay_args) for _ in range(measurements)]

    payload = samples[0]["payload"]
    violations: list[str] = []
    for sample in samples:
        if sample["exit_code"] != 0:
            violations.append(f"{models}-model PR replay returned exit code {sample['exit_code']}")
        if sample["payload"]["metrics"] != payload["metrics"]:
            violations.append(f"{models}-model PR replay metrics changed between measured runs")
        if sample["payload"].get("pr_summary") != payload.get("pr_summary"):
            violations.append(f"{models}-model PR replay summary changed between measured runs")

    summary = payload.get("pr_summary") or {}
    changed_files = summary.get("changed_files") or []
    delta = summary.get("finding_delta") or {}
    context = payload.get("context") or {}
    context_sql = (context.get("counts") or {}).get("sql", 0)
    if not any(str(path).endswith("models/generated/model_0000.sql") for path in changed_files):
        violations.append("PR replay did not report the committed model change")
    if context_sql < models - 1:
        violations.append(
            f"PR replay processed only {context_sql} unchanged SQL files; expected {models - 1}"
        )
    if delta.get("resolved", 0) < 1:
        violations.append("PR replay did not resolve the expected base-branch finding")

    return {
        "target": f"synthetic-pr/{models}",
        "models": models,
        "base": "HEAD~1",
        **summarize_measurements(samples),
        "changed_files": changed_files,
        "finding_delta": delta,
        "base_context_sql": context_sql,
        "violations": violations,
    }


def pr_replay_threshold_violations(
    replay: dict[str, Any],
    *,
    max_median_ms: int,
    max_runtime_ms: int,
    max_rss_bytes: int,
) -> list[str]:
    violations = list(replay.get("violations", []))
    if replay["runtime_median_ms"] > max_median_ms:
        violations.append(
            f"PR replay median runtime {replay['runtime_median_ms']}ms exceeded {max_median_ms}ms"
        )
    if replay["runtime_max_ms"] > max_runtime_ms:
        violations.append(
            f"PR replay max runtime {replay['runtime_max_ms']}ms exceeded {max_runtime_ms}ms"
        )
    if replay["max_rss_bytes"] > max_rss_bytes:
        violations.append(
            f"PR replay max RSS {replay['max_rss_bytes']} exceeded {max_rss_bytes}"
        )
    return violations


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
    parser.add_argument("--pr-measurements", type=int, default=2)
    parser.add_argument(
        "--pr-max-median-seconds",
        type=float,
        default=PR_REPLAY_MEDIAN_MS / 1000,
    )
    parser.add_argument(
        "--pr-max-seconds",
        type=float,
        default=PR_REPLAY_MAX_MS / 1000,
    )
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
        "pr_replay_max_runtime_median_ms": int(args.pr_max_median_seconds * 1000),
        "pr_replay_max_runtime_max_ms": int(args.pr_max_seconds * 1000),
    }
    report: dict[str, Any] = {
        "version": 4,
        "environment": environment_metadata(),
        "thresholds": thresholds,
        "targets": {},
        "growth": None,
        "max_rss_bytes": None,
    }

    violations = []
    if args.measurements < 1:
        violations.append("--measurements must be at least one")
    if args.pr_measurements < 1:
        violations.append("--pr-measurements must be at least one")
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
        replay = run_pr_replay_target(args.models, args.pr_measurements)
        report["targets"]["pr_replay"] = replay
        violations.extend(
            pr_replay_threshold_violations(
                replay,
                max_median_ms=thresholds["pr_replay_max_runtime_median_ms"],
                max_runtime_ms=thresholds["pr_replay_max_runtime_max_ms"],
                max_rss_bytes=args.max_rss_bytes,
            )
        )
        report["max_rss_bytes"] = max(
            baseline["max_rss_bytes"],
            release["max_rss_bytes"],
            replay["max_rss_bytes"],
        )
    except Exception as exc:
        violations = [f"scale execution failed: {exc}"]

    status = write_report(args.report, report, violations)
    if status:
        for violation in violations:
            print(f"FAIL {violation}", file=sys.stderr)
        return status

    release = report["targets"]["release"]
    replay = report["targets"]["pr_replay"]
    print(
        "scale gate passed: "
        f"{release['models']} models, median {release['runtime_median_ms']}ms, "
        f"max {release['runtime_max_ms']}ms, max RSS {report['max_rss_bytes']} bytes, "
        f"growth ratio {report['growth']['per_model_runtime_ratio']:.3f}; "
        f"PR replay median {replay['runtime_median_ms']}ms, "
        f"max {replay['runtime_max_ms']}ms"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
