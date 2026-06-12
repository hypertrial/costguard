#!/usr/bin/env python3
"""Run Costguard benchmarks against vendored fixtures or external dbt repos."""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import time
from pathlib import Path
from typing import Any

try:
    import tomllib
except ModuleNotFoundError:
    import tomli as tomllib  # type: ignore[no-redef]


ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))

from costguard_tooling import costguard_binary  # noqa: E402
from dbt_compile_for_costguard import compile_dbt_repo, write_json  # noqa: E402
FIXTURES = ROOT / "tests" / "fixtures"
BASELINES = ROOT / "tests" / "benchmarks" / "baselines"
REPORTS = ROOT / "tests" / "benchmarks" / "reports"
REPOS_TOML = ROOT / "tests" / "benchmarks" / "repos.toml"


def load_repos() -> list[dict[str, Any]]:
    data = tomllib.loads(REPOS_TOML.read_text(encoding="utf-8"))
    return data.get("repo", [])


def repo_by_name(name: str) -> dict[str, Any]:
    for repo in load_repos():
        if repo["name"] == name:
            return repo
    raise SystemExit(f"unknown repo '{name}' in {REPOS_TOML}")


def baseline_path(target: str) -> Path:
    safe = target.replace("/", "__")
    return BASELINES / f"{safe}.json"


def run_costguard(
    workdir: Path,
    *,
    warehouse: str,
    scan_paths: list[str],
    fail_on: str,
    manifest: Path | None = None,
) -> dict[str, Any]:
    cmd = [
        str(costguard_binary()),
        "scan",
        "--warehouse",
        warehouse,
        "--fail-on",
        fail_on,
        "--format",
        "json",
    ]
    if manifest is not None:
        if manifest.is_absolute():
            manifest_arg = manifest.relative_to(workdir) if manifest.is_relative_to(workdir) else manifest
        else:
            manifest_arg = manifest
        cmd.extend(["--manifest", str(manifest_arg)])
    cmd.extend(scan_paths)

    started = time.monotonic()
    completed = subprocess.run(
        cmd,
        cwd=workdir,
        capture_output=True,
        text=True,
        check=False,
    )
    elapsed_ms = int((time.monotonic() - started) * 1000)

    if completed.returncode not in (0, 1):
        raise SystemExit(
            f"costguard scan failed (exit {completed.returncode}):\n"
            f"{completed.stderr.strip()}"
        )

    try:
        payload = json.loads(completed.stdout)
    except json.JSONDecodeError as exc:
        raise SystemExit(
            f"failed to parse costguard JSON output: {exc}\nstdout:\n{completed.stdout}"
        ) from exc

    metrics = payload.get("metrics")
    if metrics is None:
        raise SystemExit("costguard JSON output missing 'metrics'")

    return {
        "exit_code": completed.returncode,
        "runtime_ms": elapsed_ms,
        "metrics": metrics,
        "diagnostics_count": len(payload.get("diagnostics", [])),
    }


def clone_repo(repo: dict[str, Any], cache_dir: Path) -> Path:
    checkout = cache_dir / repo["name"]
    checkout.parent.mkdir(parents=True, exist_ok=True)
    if not checkout.exists():
        subprocess.run(
            [
                "git",
                "clone",
                "--filter=blob:none",
                "--no-checkout",
                repo["url"],
                str(checkout),
            ],
            check=True,
            capture_output=True,
            text=True,
        )
    subprocess.run(
        ["git", "fetch", "origin", repo["commit"], "--depth", "1"],
        cwd=checkout,
        check=True,
        capture_output=True,
        text=True,
    )
    subprocess.run(
        ["git", "checkout", "--force", "FETCH_HEAD"],
        cwd=checkout,
        check=True,
        capture_output=True,
        text=True,
    )
    return checkout


def build_report(
    target: str,
    *,
    warehouse: str,
    scan_result: dict[str, Any],
    kind: str,
    compile_cache: str | None = None,
) -> dict[str, Any]:
    report = {
        "version": 1,
        "target": target,
        "kind": kind,
        "warehouse": warehouse,
        "metrics": scan_result["metrics"],
        "runtime_ms": scan_result["runtime_ms"],
        "exit_code": scan_result["exit_code"],
        "diagnostics_count": scan_result["diagnostics_count"],
    }
    if compile_cache is not None:
        report["compile_cache"] = compile_cache
    return report


def compare_report(report: dict[str, Any], baseline: dict[str, Any]) -> list[str]:
    errors: list[str] = []
    thresholds = baseline.get("thresholds", {})
    actual = report["metrics"]
    expected = baseline.get("metrics", {})

    max_parse_failures = thresholds.get("max_sql_parse_failures")
    if max_parse_failures is not None and actual["sql_parse_failures"] > max_parse_failures:
        errors.append(
            "sql_parse_failures "
            f"{actual['sql_parse_failures']} > allowed {max_parse_failures}"
        )

    parse_total = actual.get("sql_parse_total", 0)
    if parse_total:
        rate = actual["sql_parse_failures"] / parse_total
        max_rate = thresholds.get("max_parse_failure_rate")
        if max_rate is not None and rate > max_rate:
            errors.append(f"parse failure rate {rate:.3f} > allowed {max_rate:.3f}")

    baseline_failures = expected.get("sql_parse_failures", 0)
    delta = thresholds.get("max_parse_failure_delta")
    if delta is not None:
        if actual["sql_parse_failures"] > baseline_failures + delta:
            errors.append(
                "sql_parse_failures regressed "
                f"{actual['sql_parse_failures']} > baseline {baseline_failures} + {delta}"
            )
    elif max_parse_failures is None:
        if actual["sql_parse_failures"] > baseline_failures:
            errors.append(
                "sql_parse_failures regressed "
                f"{actual['sql_parse_failures']} > baseline {baseline_failures}"
            )

    max_compiled_failures = thresholds.get("max_sql_parse_compiled_failures")
    if max_compiled_failures is not None:
        actual_compiled = actual.get("sql_parse_compiled_failures", 0)
        if actual_compiled > max_compiled_failures:
            errors.append(
                "sql_parse_compiled_failures "
                f"{actual_compiled} > allowed {max_compiled_failures}"
            )

    baseline_compiled_failures = expected.get("sql_parse_compiled_failures", 0)
    compiled_delta = thresholds.get("max_sql_parse_compiled_failure_delta")
    if compiled_delta is not None:
        actual_compiled = actual.get("sql_parse_compiled_failures", 0)
        if actual_compiled > baseline_compiled_failures + compiled_delta:
            errors.append(
                "sql_parse_compiled_failures regressed "
                f"{actual_compiled} > baseline {baseline_compiled_failures} + {compiled_delta}"
            )
    elif max_compiled_failures is None and baseline_compiled_failures:
        actual_compiled = actual.get("sql_parse_compiled_failures", 0)
        if actual_compiled > baseline_compiled_failures:
            errors.append(
                "sql_parse_compiled_failures regressed "
                f"{actual_compiled} > baseline {baseline_compiled_failures}"
            )

    expected_rules = baseline.get("expect_rules", thresholds.get("expect_rules", []))
    actual_rules = set(actual.get("diagnostics_by_rule", {}))
    for rule in expected_rules:
        if rule not in actual_rules:
            errors.append(f"missing expected rule {rule}")

    forbid_rules = baseline.get("forbid_rules", thresholds.get("forbid_rules", []))
    for rule in forbid_rules:
        if rule in actual_rules:
            errors.append(f"forbidden rule present: {rule}")

    exact_rules = thresholds.get("exact_diagnostics_by_rule")
    if exact_rules is not None:
        for rule, count in exact_rules.items():
            actual_count = actual.get("diagnostics_by_rule", {}).get(rule, 0)
            if actual_count != count:
                errors.append(
                    f"rule {rule} count {actual_count} != expected {count}"
                )

    max_rules = thresholds.get("max_diagnostics_by_rule", {})
    for rule, ceiling in max_rules.items():
        actual_count = actual.get("diagnostics_by_rule", {}).get(rule, 0)
        if actual_count > ceiling:
            errors.append(
                f"rule {rule} count {actual_count} > max {ceiling}"
            )

    max_runtime_ms = thresholds.get("max_runtime_ms")
    if max_runtime_ms is not None:
        actual_runtime = report.get("runtime_ms", 0)
        if actual_runtime > max_runtime_ms:
            errors.append(
                f"runtime_ms {actual_runtime} > allowed {max_runtime_ms}"
            )

    return errors


def run_fixture(
    fixture: str,
    *,
    update_baseline: bool,
    warehouse: str | None,
) -> int:
    fixture_path = FIXTURES / fixture
    if not fixture_path.is_dir():
        raise SystemExit(f"fixture not found: {fixture_path}")

    target = fixture.replace("\\", "/")
    wh = warehouse or "generic"
    manifest = fixture_path / "target" / "manifest.json"
    scan_result = run_costguard(
        ROOT,
        warehouse=wh,
        scan_paths=[str(fixture_path.relative_to(ROOT))],
        fail_on="critical",
        manifest=manifest if manifest.exists() else None,
    )
    report = build_report(target, warehouse=wh, scan_result=scan_result, kind="vendored")
    write_json(REPORTS / f"{target.replace('/', '__')}.json", report)

    baseline_file = baseline_path(target)
    if update_baseline or not baseline_file.exists():
        baseline = {
            "version": 1,
            "target": target,
            "kind": "vendored",
            "warehouse": wh,
            "metrics": report["metrics"],
            "expect_rules": [],
            "forbid_rules": ["SQLCOST005"] if "spellbook" not in target else [],
            "thresholds": {
                "max_sql_parse_failures": report["metrics"]["sql_parse_failures"],
                "exact_diagnostics_by_rule": report["metrics"]["diagnostics_by_rule"],
            },
        }
        write_json(baseline_file, baseline)
        print(f"updated baseline: {baseline_file}")
        return 0

    baseline = json.loads(baseline_file.read_text(encoding="utf-8"))
    errors = compare_report(report, baseline)
    if errors:
        for error in errors:
            print(f"FAIL {target}: {error}", file=sys.stderr)
        return 1

    print(f"PASS {target} ({report['runtime_ms']} ms)")
    return 0


def run_external(
    repo_name: str,
    *,
    update_baseline: bool,
    cache_dir: Path,
    smoke: bool = False,
    force_compile: bool = False,
) -> int:
    repo = repo_by_name(repo_name)
    checkout = clone_repo(repo, cache_dir)
    compile_cache = compile_dbt_repo(
        checkout,
        repo,
        cache_dir=cache_dir,
        smoke=smoke,
        force_compile=force_compile,
    )
    if smoke:
        scan_paths = repo.get("smoke_scan_paths", repo.get("scan_paths", ["."]))
        target = f"external/{repo_name}_smoke"
    else:
        scan_paths = repo.get("scan_paths", ["."])
        target = f"external/{repo_name}"
    manifest = checkout / "target" / "manifest.json"
    scan_result = run_costguard(
        checkout,
        warehouse=repo.get("warehouse", "generic"),
        scan_paths=scan_paths,
        fail_on=repo.get("fail_on", "critical"),
        manifest=manifest if manifest.exists() else None,
    )
    report = build_report(
        target,
        warehouse=repo.get("warehouse", "generic"),
        scan_result=scan_result,
        kind="external",
        compile_cache=compile_cache,
    )
    report["commit"] = repo["commit"]
    write_json(REPORTS / f"{target.replace('/', '__')}.json", report)

    baseline_file = baseline_path(target)
    if update_baseline or not baseline_file.exists():
        existing_thresholds = {}
        if baseline_file.exists():
            existing_thresholds = json.loads(baseline_file.read_text(encoding="utf-8")).get(
                "thresholds", {}
            )
        baseline = {
            "version": 1,
            "target": target,
            "kind": "external",
            "warehouse": repo.get("warehouse", "generic"),
            "commit": repo["commit"],
            "metrics": report["metrics"],
            "runtime_ms": report["runtime_ms"],
            "thresholds": {
                "max_parse_failure_delta": existing_thresholds.get("max_parse_failure_delta", 50),
                "max_sql_parse_compiled_failures": existing_thresholds.get(
                    "max_sql_parse_compiled_failures", 0
                ),
                "max_diagnostics_by_rule": existing_thresholds.get(
                    "max_diagnostics_by_rule", {}
                ),
                "max_runtime_ms": existing_thresholds.get(
                    "max_runtime_ms",
                    max(int(report["runtime_ms"] * 1.25), report["runtime_ms"] + 1000),
                ),
            },
        }
        parse_total = report["metrics"].get("sql_parse_total", 0)
        if parse_total:
            rate = report["metrics"]["sql_parse_failures"] / parse_total
            configured_rate = repo.get("max_parse_failure_rate")
            if configured_rate is not None:
                baseline["thresholds"]["max_parse_failure_rate"] = max(configured_rate, rate)
            elif "max_parse_failure_rate" in existing_thresholds:
                baseline["thresholds"]["max_parse_failure_rate"] = existing_thresholds[
                    "max_parse_failure_rate"
                ]
        max_rules = baseline["thresholds"].get("max_diagnostics_by_rule", {})
        if max_rules:
            for rule, _ in list(max_rules.items()):
                actual = report["metrics"].get("diagnostics_by_rule", {}).get(rule)
                if actual is not None:
                    max_rules[rule] = actual
            baseline["thresholds"]["max_diagnostics_by_rule"] = max_rules
        write_json(baseline_file, baseline)
        print(f"updated baseline: {baseline_file}")
        return 0

    baseline = json.loads(baseline_file.read_text(encoding="utf-8"))
    errors = compare_report(report, baseline)
    if errors:
        for error in errors:
            print(f"FAIL {target}: {error}", file=sys.stderr)
        return 1

    print(
        f"PASS {target} ({report['runtime_ms']} ms, "
        f"{report['metrics']['sql_parse_failures']} parse failures"
        f"{f', compile_cache={compile_cache}' if compile_cache != 'skip' else ''})"
    )
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    group = parser.add_mutually_exclusive_group(required=True)
    group.add_argument("--fixture", help="vendored fixture path under tests/fixtures/")
    group.add_argument("--repo", help="external repo name from tests/benchmarks/repos.toml")
    group.add_argument(
        "--all-vendored",
        action="store_true",
        help="run all vendored real_world fixtures",
    )
    parser.add_argument("--warehouse", help="override warehouse platform")
    parser.add_argument(
        "--cache",
        default=os.environ.get(
            "COSTGUARD_BENCHMARK_CACHE",
            str(Path.home() / ".cache" / "costguard" / "benchmarks"),
        ),
    )
    parser.add_argument(
        "--update-baseline",
        action="store_true",
        help="write or refresh baseline JSON",
    )
    parser.add_argument(
        "--smoke",
        action="store_true",
        help="run repo smoke profile (requires smoke_* keys in repos.toml)",
    )
    parser.add_argument(
        "--force-compile",
        action="store_true",
        help="bypass cached dbt manifest and recompile",
    )
    args = parser.parse_args()

    if args.all_vendored:
        fixtures = [
            "real_world/jaffle_snippets",
            "real_world/spellbook_snippets",
            "real_world/snowflake_snippets",
            "real_world/manifest_graph",
            "real_world/multi_dbt",
        ]
        return max(
            run_fixture(fixture, update_baseline=args.update_baseline, warehouse=args.warehouse)
            for fixture in fixtures
        )

    if args.fixture:
        return run_fixture(
            args.fixture,
            update_baseline=args.update_baseline,
            warehouse=args.warehouse,
        )

    return run_external(
        args.repo,
        update_baseline=args.update_baseline,
        cache_dir=Path(args.cache),
        smoke=args.smoke,
        force_compile=args.force_compile,
    )


if __name__ == "__main__":
    raise SystemExit(main())
