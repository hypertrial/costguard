#!/usr/bin/env python3
"""Bucket Costguard rule diagnostics for external-repo triage."""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
from collections import Counter, defaultdict
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))

try:
    import tomllib
except ModuleNotFoundError:
    import tomli as tomllib  # type: ignore[no-redef]

REPOS_TOML = ROOT / "tests" / "benchmarks" / "repos.toml"

CROSS_JOIN_RE = re.compile(r"(?i)\bcross\s+join\b")
CROSS_JOIN_UNNEST_RE = re.compile(r"(?i)\bcross\s+join\s+(?:unnest|table)\s*\(")
FROM_COMMA_RE = re.compile(r"(?i)\bfrom\b")


def mask_literals_and_comments(text: str) -> str:
    output: list[str] = []
    i = 0
    while i < len(text):
        if text.startswith("--", i):
            end = text.find("\n", i)
            if end == -1:
                output.append(" " * (len(text) - i))
                break
            output.append(" " * (end - i))
            output.append("\n")
            i = end + 1
            continue
        if text[i] == "'":
            j = i + 1
            while j < len(text):
                if text[j] == "'" and text[j - 1 : j + 1] != "''":
                    break
                j += 1
            output.append(" " * (j - i + 1))
            i = j + 1
            continue
        output.append(text[i])
        i += 1
    return "".join(output)


def cross_join_only_in_literals(text: str) -> bool:
    if not CROSS_JOIN_RE.search(text):
        return False
    masked = mask_literals_and_comments(text)
    return not CROSS_JOIN_RE.search(masked)


def has_top_level_comma_after_from(masked: str) -> bool:
    for match in FROM_COMMA_RE.finditer(masked):
        start = match.end()
        depth = 0
        i = start
        while i < len(masked):
            ch = masked[i]
            if ch == "(":
                depth += 1
            elif ch == ")":
                depth = max(depth - 1, 0)
            elif ch == "," and depth == 0:
                tail = masked[i + 1 : i + 80].lstrip()
                if tail and tail[0].isalpha():
                    return True
                break
            lower_tail = masked[i : i + 12].lower()
            if depth == 0 and lower_tail.startswith(
                (" where ", " group by ", " order by ", " limit ", "\nwhere ")
            ):
                break
            i += 1
    return False


def looks_like_subquery_comma_fp(masked: str) -> bool:
    return bool(re.search(r"(?is)\bfrom\s*\(\s*select[^)]*,", masked))


def classify_sqlcost012(sql: str) -> str:
    if cross_join_only_in_literals(sql):
        return "string_literal_fp"

    masked = mask_literals_and_comments(sql)
    lower = masked.lower()

    if CROSS_JOIN_UNNEST_RE.search(lower):
        return "cross_join_unnest"

    if looks_like_subquery_comma_fp(masked):
        return "subquery_comma_fp"

    if CROSS_JOIN_RE.search(lower):
        return "cross_join_explicit"

    if has_top_level_comma_after_from(masked):
        return "comma_join"

    return "other"


def costguard_binary() -> Path:
    target_dir = Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target"))
    binary = target_dir / "debug" / "costguard"
    if not binary.exists():
        build = subprocess.run(
            ["cargo", "build", "-q", "-p", "costguard-cli"],
            cwd=ROOT,
            check=False,
        )
        if build.returncode != 0:
            raise SystemExit("failed to build costguard-cli")
    if not binary.exists():
        raise SystemExit(f"costguard binary not found at {binary}")
    return binary


def load_repo(name: str) -> dict[str, Any]:
    data = tomllib.loads(REPOS_TOML.read_text(encoding="utf-8"))
    for repo in data.get("repo", []):
        if repo["name"] == name:
            return repo
    raise SystemExit(f"unknown repo '{name}' in {REPOS_TOML}")


def run_scan(
    checkout: Path,
    *,
    warehouse: str,
    scan_paths: list[str],
    manifest: Path,
) -> dict[str, Any]:
    cmd = [
        str(costguard_binary()),
        "scan",
        "--warehouse",
        warehouse,
        "--fail-on",
        "critical",
        "--format",
        "json",
        "--manifest",
        str(manifest.relative_to(checkout)),
        *scan_paths,
    ]
    completed = subprocess.run(
        cmd,
        cwd=checkout,
        capture_output=True,
        text=True,
        check=False,
    )
    if completed.returncode not in (0, 1):
        raise SystemExit(
            f"costguard scan failed (exit {completed.returncode}):\n{completed.stderr}"
        )
    return json.loads(completed.stdout)


def load_manifest_sql(manifest_path: Path) -> dict[str, str]:
    payload = json.loads(manifest_path.read_text(encoding="utf-8"))
    compiled: dict[str, str] = {}
    for node in payload.get("nodes", {}).values():
        if node.get("resource_type") != "model":
            continue
        path = node.get("original_file_path") or node.get("path")
        code = node.get("compiled_code")
        if path and code:
            compiled[path.replace("\\", "/")] = code
    return compiled


def read_sql_for_diagnostic(
    checkout: Path,
    diagnostic: dict[str, Any],
    compiled_by_path: dict[str, str],
) -> str:
    rel_path = diagnostic.get("path", "").replace("\\", "/")
    if rel_path in compiled_by_path:
        return compiled_by_path[rel_path]
    file_path = checkout / rel_path
    if file_path.exists():
        return file_path.read_text(encoding="utf-8", errors="replace")
    return ""


def bucket_diagnostics(
    diagnostics: list[dict[str, Any]],
    checkout: Path,
    compiled_by_path: dict[str, str],
    rule_id: str,
    limit: int | None,
) -> dict[str, Any]:
    filtered = [d for d in diagnostics if d.get("rule_id") == rule_id]
    if limit is not None:
        filtered = filtered[:limit]

    buckets: Counter[str] = Counter()
    examples: dict[str, list[dict[str, str]]] = defaultdict(list)

    for diagnostic in filtered:
        sql = read_sql_for_diagnostic(checkout, diagnostic, compiled_by_path)
        bucket = classify_sqlcost012(sql) if rule_id == "SQLCOST012" else "other"
        buckets[bucket] += 1
        if len(examples[bucket]) < 5:
            rel_path = str(diagnostic.get("path", ""))
            line = str(diagnostic.get("line", ""))
            snippet = sql[max(0, (diagnostic.get("line", 1) - 1) * 40) :][:240].replace("\n", " ")
            examples[bucket].append(
                {"path": rel_path, "line": line, "snippet": snippet}
            )

    return {
        "rule_id": rule_id,
        "total": len(filtered),
        "buckets": dict(buckets),
        "examples": dict(examples),
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--cache",
        type=Path,
        default=Path.home() / ".cache" / "costguard" / "benchmarks",
        help="Benchmark cache root",
    )
    parser.add_argument(
        "--repo",
        default="spellbook",
        help="Repo name from tests/benchmarks/repos.toml",
    )
    parser.add_argument("--rule", default="SQLCOST012", help="Rule id to bucket")
    parser.add_argument("--limit", type=int, default=None, help="Max diagnostics to classify")
    parser.add_argument("--json-out", type=Path, default=None, help="Write JSON report")
    args = parser.parse_args()

    repo = load_repo(args.repo)
    checkout = args.cache / args.repo
    manifest = checkout / "target" / "manifest.json"
    if not manifest.exists():
        raise SystemExit(
            f"missing manifest at {manifest}; run benchmark_external_repo.py --repo {args.repo} first"
        )

    payload = run_scan(
        checkout,
        warehouse=repo.get("warehouse", "generic"),
        scan_paths=repo.get("scan_paths", ["."]),
        manifest=manifest,
    )
    compiled_by_path = load_manifest_sql(manifest)
    report = bucket_diagnostics(
        payload.get("diagnostics", []),
        checkout,
        compiled_by_path,
        args.rule,
        args.limit,
    )

    print(f"Rule {args.rule}: {report['total']} diagnostics")
    for bucket, count in sorted(report["buckets"].items(), key=lambda item: -item[1]):
        print(f"  {bucket}: {count}")
        for example in report["examples"].get(bucket, []):
            print(f"    - {example['path']}:{example['line']} {example['snippet'][:100]}")

    if args.json_out is not None:
        args.json_out.parent.mkdir(parents=True, exist_ok=True)
        args.json_out.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
        print(f"wrote {args.json_out}")


if __name__ == "__main__":
    main()
