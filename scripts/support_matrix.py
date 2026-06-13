#!/usr/bin/env python3
"""Validate and execute the pinned enterprise support matrix."""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CONFIG = ROOT / "tests" / "benchmarks" / "repos.toml"
BASELINES = ROOT / "tests" / "benchmarks" / "baselines"
COMMIT_RE = re.compile(r"^[0-9a-f]{40}$")
SUPPORTED_WAREHOUSES = {
    "generic",
    "snowflake",
    "bigquery",
    "databricks",
    "redshift",
    "postgres",
    "duckdb",
    "trino",
}


def load_matrix(path: Path = CONFIG) -> list[dict[str, object]]:
    data = tomllib.loads(path.read_text(encoding="utf-8"))
    return data.get("repo", [])


def validate_matrix(repos: list[dict[str, object]]) -> list[str]:
    errors: list[str] = []
    names: set[str] = set()
    for repo in repos:
        name = str(repo.get("name", ""))
        prefix = name or "<unnamed>"
        if not name or name in names:
            errors.append(f"{prefix}: name must be non-empty and unique")
        names.add(name)
        url = str(repo.get("url", ""))
        if not url.startswith("https://github.com/") or not url.endswith(".git"):
            errors.append(f"{prefix}: url must be an HTTPS GitHub clone URL")
        if not COMMIT_RE.fullmatch(str(repo.get("commit", ""))):
            errors.append(f"{prefix}: commit must be a full lowercase SHA")
        if repo.get("warehouse") not in SUPPORTED_WAREHOUSES:
            errors.append(f"{prefix}: unsupported warehouse {repo.get('warehouse')!r}")
        paths = repo.get("scan_paths")
        if not isinstance(paths, list) or not paths or not all(isinstance(path, str) for path in paths):
            errors.append(f"{prefix}: scan_paths must be a non-empty string array")
        if repo.get("compile_dbt") and not repo.get("dbt_adapter"):
            errors.append(f"{prefix}: compile_dbt requires dbt_adapter")
        if bool(repo.get("required", True)):
            baseline = BASELINES / f"external__{name}.json"
            if not baseline.is_file():
                errors.append(f"{prefix}: required benchmark baseline is missing: {baseline.name}")
    return errors


def run(command: list[str]) -> None:
    print("+", " ".join(command), flush=True)
    subprocess.run(command, cwd=ROOT, check=True)


def run_benchmarks(repos: list[dict[str, object]]) -> None:
    run([sys.executable, "scripts/benchmark_external_repo.py", "--all-vendored"])
    for repo in repos:
        if not bool(repo.get("required", True)):
            continue
        run(
            [
                sys.executable,
                "scripts/benchmark_external_repo.py",
                "--repo",
                str(repo["name"]),
                "--force-compile",
            ]
        )
    run(
        [
            sys.executable,
            "scripts/precision_triage.py",
            "--repo",
            "spellbook",
            "--json-out",
            "tests/benchmarks/reports/precision__spellbook.json",
        ]
    )


def report(repos: list[dict[str, object]]) -> dict[str, object]:
    return {
        "schema_version": 1,
        "repositories": [
            {
                "name": repo["name"],
                "commit": repo["commit"],
                "warehouse": repo["warehouse"],
                "required": bool(repo.get("required", True)),
                "baseline": (BASELINES / f"external__{repo['name']}.json").is_file(),
            }
            for repo in repos
        ],
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--verify", action="store_true")
    parser.add_argument("--run-benchmarks", action="store_true")
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args()
    repos = load_matrix()
    errors = validate_matrix(repos)
    if args.verify and errors:
        for error in errors:
            print(f"FAIL {error}", file=sys.stderr)
        return 1
    if args.run_benchmarks:
        run_benchmarks(repos)
    if args.json:
        print(json.dumps(report(repos), indent=2, sort_keys=True))
    else:
        for repo in report(repos)["repositories"]:
            status = "required" if repo["required"] else "observational"
            print(f"{repo['name']}: {repo['warehouse']} {repo['commit']} ({status})")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
