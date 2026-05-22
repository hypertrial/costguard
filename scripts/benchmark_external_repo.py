#!/usr/bin/env python3
"""Run Costguard benchmarks against vendored fixtures or external dbt repos."""

from __future__ import annotations

import argparse
import json
import os
import re
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


def costguard_binary() -> Path:
    target_dir = Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target"))
    binary = target_dir / "debug" / "costguard"
    build = subprocess.run(
        ["cargo", "build", "-q", "-p", "costguard-cli"],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=False,
    )
    if build.returncode != 0:
        raise SystemExit(f"failed to build costguard-cli:\n{build.stderr}")
    if not binary.exists():
        raise SystemExit(f"costguard binary not found at {binary}")
    return binary


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


def read_dbt_profile_name(project_dir: Path) -> str:
    text = (project_dir / "dbt_project.yml").read_text(encoding="utf-8")
    match = re.search(r"^profile:\s*['\"]?(\w+)['\"]?", text, re.MULTILINE)
    return match.group(1) if match else "costguard"


def write_dummy_profiles(
    profiles_dir: Path,
    *,
    profile_name: str,
    target: str,
    profile_type: str,
) -> None:
    profiles_dir.mkdir(parents=True, exist_ok=True)
    profiles_file = profiles_dir / "profiles.yml"
    if profiles_file.exists():
        return
    profiles_file.write_text(
        f"""{profile_name}:
  target: {target}
  outputs:
    {target}:
      type: {profile_type}
      host: localhost
      port: 8080
      user: costguard
      database: costguard
      schema: costguard
""",
        encoding="utf-8",
    )


def venv_python() -> str:
    for candidate in ("python3.11", "python3.12", sys.executable):
        completed = subprocess.run(
            [candidate, "-c", "import sys; raise SystemExit(0 if sys.version_info < (3, 14) else 1)"],
            capture_output=True,
            text=True,
            check=False,
        )
        if completed.returncode == 0:
            return candidate
    return sys.executable


def dbt_tools(cache_dir: Path, adapter: str) -> tuple[Path, Path]:
    venv_dir = cache_dir / ".dbt-venv"
    if not venv_dir.exists():
        created = subprocess.run(
            [venv_python(), "-m", "venv", str(venv_dir)],
            capture_output=True,
            text=True,
            check=False,
        )
        if created.returncode != 0:
            raise SystemExit(f"failed to create dbt venv:\n{created.stderr}")

    pip = venv_dir / "bin" / "pip"
    dbt = venv_dir / "bin" / "dbt"
    install = subprocess.run(
        [str(pip), "install", "--upgrade", "pip", adapter],
        capture_output=True,
        text=True,
        check=False,
    )
    if install.returncode != 0:
        raise SystemExit(f"failed to install {adapter}:\n{install.stderr}")
    return pip, dbt


def compile_dbt_project(
    checkout: Path,
    project_dir: Path,
    repo: dict[str, Any],
    *,
    dbt: Path,
) -> Path:
    if not (project_dir / "dbt_project.yml").exists():
        raise SystemExit(f"compile_dbt enabled but no dbt_project.yml in {project_dir}")

    target = repo.get("dbt_target", "dev")
    if (project_dir / "profiles.yml").exists():
        profiles_dir = project_dir
    else:
        profiles_rel = repo.get("dbt_profiles_dir", ".")
        profiles_dir = (checkout / profiles_rel).resolve()
        profile_name = read_dbt_profile_name(project_dir)
        write_dummy_profiles(
            profiles_dir,
            profile_name=profile_name,
            target=target,
            profile_type=repo.get("dbt_profile_type", "trino"),
        )

    env = os.environ.copy()
    env["DBT_PROFILES_DIR"] = str(profiles_dir)

    deps = subprocess.run(
        [str(dbt), "deps", "--project-dir", str(project_dir)],
        cwd=checkout,
        env=env,
        capture_output=True,
        text=True,
        check=False,
    )
    if deps.returncode != 0:
        print(f"warning: dbt deps failed for {project_dir}:\n{deps.stderr}", file=sys.stderr)

    compile_proc = subprocess.run(
        [str(dbt), "compile", "--project-dir", str(project_dir), "--target", target],
        cwd=checkout,
        env=env,
        capture_output=True,
        text=True,
        check=False,
    )
    if compile_proc.returncode != 0:
        raise SystemExit(
            f"dbt compile failed for {project_dir} (exit {compile_proc.returncode}):\n"
            f"{compile_proc.stderr.strip()}"
        )

    manifest = project_dir / "target" / "manifest.json"
    if not manifest.exists():
        raise SystemExit(f"dbt compile succeeded but manifest missing at {manifest}")
    return manifest


def merge_manifests(entries: list[tuple[Path, str]], output: Path) -> None:
    merged: dict[str, Any] = {"nodes": {}, "sources": {}, "exposures": {}}
    for manifest_path, prefix in entries:
        data = json.loads(manifest_path.read_text(encoding="utf-8"))
        for section in ("nodes", "sources", "exposures"):
            for key, value in data.get(section, {}).items():
                if section == "nodes" and value.get("resource_type") == "model":
                    value = dict(value)
                    rel_path = value.get("original_file_path") or value.get("path")
                    if rel_path and prefix:
                        value["original_file_path"] = str(Path(prefix) / rel_path)
                merged[section][key] = value
    output.parent.mkdir(parents=True, exist_ok=True)
    write_json(output, merged)


def compile_dbt_repo(checkout: Path, repo: dict[str, Any], *, cache_dir: Path) -> None:
    if not repo.get("compile_dbt", False):
        return

    adapter = repo.get("dbt_adapter", "dbt-trino")
    _, dbt = dbt_tools(cache_dir, adapter)

    compile_dirs = repo.get("dbt_compile_dirs")
    if compile_dirs:
        manifests: list[tuple[Path, str]] = []
        for rel in compile_dirs:
            project_dir = (checkout / rel).resolve()
            manifest = compile_dbt_project(checkout, project_dir, repo, dbt=dbt)
            manifests.append((manifest, rel))
        merge_manifests(manifests, checkout / "target" / "manifest.json")
        return

    project_rel = repo.get("dbt_project_dir", ".")
    project_dir = (checkout / project_rel).resolve()
    manifest = compile_dbt_project(checkout, project_dir, repo, dbt=dbt)
    if project_rel != ".":
        merge_manifests([(manifest, project_rel)], checkout / "target" / "manifest.json")


def build_report(
    target: str,
    *,
    warehouse: str,
    scan_result: dict[str, Any],
    kind: str,
) -> dict[str, Any]:
    return {
        "version": 1,
        "target": target,
        "kind": kind,
        "warehouse": warehouse,
        "metrics": scan_result["metrics"],
        "runtime_ms": scan_result["runtime_ms"],
        "exit_code": scan_result["exit_code"],
        "diagnostics_count": scan_result["diagnostics_count"],
    }


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

    return errors


def write_json(path: Path, payload: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")


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
) -> int:
    repo = repo_by_name(repo_name)
    checkout = clone_repo(repo, cache_dir)
    compile_dbt_repo(checkout, repo, cache_dir=cache_dir)
    scan_paths = repo.get("scan_paths", ["."])
    manifest = checkout / "target" / "manifest.json"
    scan_result = run_costguard(
        checkout,
        warehouse=repo.get("warehouse", "generic"),
        scan_paths=scan_paths,
        fail_on=repo.get("fail_on", "critical"),
        manifest=manifest if manifest.exists() else None,
    )
    target = f"external/{repo_name}"
    report = build_report(
        target,
        warehouse=repo.get("warehouse", "generic"),
        scan_result=scan_result,
        kind="external",
    )
    report["commit"] = repo["commit"]
    write_json(REPORTS / f"{target.replace('/', '__')}.json", report)

    baseline_file = baseline_path(target)
    if update_baseline or not baseline_file.exists():
        baseline = {
            "version": 1,
            "target": target,
            "kind": "external",
            "warehouse": repo.get("warehouse", "generic"),
            "commit": repo["commit"],
            "metrics": report["metrics"],
            "thresholds": {
                "max_parse_failure_delta": 50,
            },
        }
        parse_total = report["metrics"].get("sql_parse_total", 0)
        if parse_total:
            rate = report["metrics"]["sql_parse_failures"] / parse_total
            configured_rate = repo.get("max_parse_failure_rate")
            if configured_rate is not None:
                baseline["thresholds"]["max_parse_failure_rate"] = max(configured_rate, rate)
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
        f"{report['metrics']['sql_parse_failures']} parse failures)"
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
    args = parser.parse_args()

    if args.all_vendored:
        fixtures = [
            "real_world/jaffle_snippets",
            "real_world/spellbook_snippets",
            "real_world/manifest_graph",
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
    )


if __name__ == "__main__":
    raise SystemExit(main())
