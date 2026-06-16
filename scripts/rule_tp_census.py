#!/usr/bin/env python3
"""Full-corpus per-rule TP/FP census across benchmark repos."""

from __future__ import annotations

import argparse
import json
import os
import sys
from collections import Counter, defaultdict
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
os.environ.setdefault("CARGO_TARGET_DIR", str(ROOT / "target"))

from benchmark_external_repo import clone_repo  # noqa: E402
from bucket_rule_diagnostics import (  # noqa: E402
    load_manifest_sql,
    load_repo,
    read_sql_for_diagnostic,
)
from costguard_tooling import load_repos, run_costguard_scan  # noqa: E402
from dbt_compile_for_costguard import compile_dbt_repo  # noqa: E402
from precision_triage import classify_diagnostic  # noqa: E402

BUILTIN_RULE_IDS = [f"SQLCOST{i:03d}" for i in range(1, 45)]
INFRASTRUCTURE_RULES = {
    "SQLCOST023",
    "SQLCOST024",
    "SQLCOST025",
    "SQLCOST026",
    "SQLCOST027",
}
DEFAULT_REPOS = ["spellbook", "jaffle-shop", "mattermost-warehouse", "data-infra"]
EVIDENCE_PATH = ROOT / "tests" / "benchmarks" / "rule_tp_evidence.json"
TP_TARGET = 20


def snippet_for(sql: str, line: int, *, width: int = 240) -> str:
    if not sql:
        return ""
    lines = sql.splitlines()
    idx = max(0, min(len(lines) - 1, line - 1))
    text = lines[idx] if lines else ""
    return text.strip()[:width]


def pass_reason(*, tp: int, fp: int, unknown: int, total: int, infrastructure: bool) -> str:
    if infrastructure:
        return "infrastructure_na"
    if fp == 0 and unknown == 0:
        if tp >= TP_TARGET:
            return f"tp>={TP_TARGET}"
        if total == 0:
            return "vacuous_clean"
        return "clean"
    if tp >= TP_TARGET:
        return f"tp>={TP_TARGET}_with_residual_fp"
    return "fail"


def rule_passes(*, tp: int, fp: int, unknown: int, infrastructure: bool) -> bool:
    if infrastructure:
        return True
    if fp == 0 and unknown == 0:
        return True
    return tp >= TP_TARGET


def scan_repo(
    repo_name: str,
    *,
    cache_dir: Path,
    force_compile: bool = False,
) -> tuple[list[dict[str, Any]], Path, dict[str, str]]:
    repo = load_repo(repo_name)
    checkout = clone_repo(repo, cache_dir)
    compile_dbt_repo(
        checkout,
        repo,
        cache_dir=cache_dir,
        smoke=False,
        force_compile=force_compile,
    )
    manifest = checkout / "target" / "manifest.json"
    compiled = load_manifest_sql(manifest) if manifest.is_file() else {}
    enable_cost = bool(repo.get("cost", False))
    payload, _ = run_costguard_scan(
        checkout,
        warehouse=repo.get("warehouse", "generic"),
        scan_paths=repo.get("scan_paths", ["."]),
        fail_on=repo.get("fail_on", "critical"),
        manifest=manifest if manifest.is_file() else None,
        cost=enable_cost,
    )
    return payload.get("diagnostics", []), checkout, compiled


def aggregate_diagnostics(
    repo_name: str,
    diagnostics: list[dict[str, Any]],
    checkout: Path,
    compiled: dict[str, str],
) -> dict[str, dict[str, Any]]:
    by_rule: dict[str, dict[str, Any]] = defaultdict(
        lambda: {
            "total": 0,
            "tp": 0,
            "fp": 0,
            "unknown": 0,
            "examples": [],
            "unknown_buckets": Counter(),
            "fp_buckets": Counter(),
        }
    )
    for diagnostic in diagnostics:
        rule_id = diagnostic.get("rule_id", "")
        if not rule_id:
            continue
        bucket, verdict = classify_diagnostic(diagnostic, checkout, compiled, repo_name)
        entry = by_rule[rule_id]
        entry["total"] += 1
        if verdict == "tp":
            entry["tp"] += 1
            if len(entry["examples"]) < TP_TARGET:
                line = int(diagnostic.get("line") or 0)
                sql = read_sql_for_diagnostic(checkout, diagnostic, compiled)
                entry["examples"].append(
                    {
                        "repo": repo_name,
                        "path": diagnostic.get("path", ""),
                        "line": line,
                        "bucket": bucket,
                        "message": diagnostic.get("message", ""),
                        "snippet": snippet_for(sql, line),
                    }
                )
        elif verdict == "fp":
            entry["fp"] += 1
            entry["fp_buckets"][f"{repo_name}:{bucket}"] += 1
        else:
            entry["unknown"] += 1
            entry["unknown_buckets"][f"{repo_name}:{bucket}"] += 1
    return by_rule


def merge_rule_stats(
    merged: dict[str, dict[str, Any]],
    repo_stats: dict[str, dict[str, Any]],
) -> None:
    for rule_id, stats in repo_stats.items():
        entry = merged.setdefault(
            rule_id,
            {
                "total": 0,
                "tp": 0,
                "fp": 0,
                "unknown": 0,
                "examples": [],
                "unknown_buckets": Counter(),
                "fp_buckets": Counter(),
            },
        )
        entry["total"] += stats["total"]
        entry["tp"] += stats["tp"]
        entry["fp"] += stats["fp"]
        entry["unknown"] += stats["unknown"]
        entry["unknown_buckets"].update(stats["unknown_buckets"])
        entry["fp_buckets"].update(stats["fp_buckets"])
        for example in stats["examples"]:
            if len(entry["examples"]) >= TP_TARGET:
                break
            entry["examples"].append(example)


def build_report(
    merged: dict[str, dict[str, Any]],
    *,
    repos: list[str],
) -> dict[str, Any]:
    rules: dict[str, dict[str, Any]] = {}
    for rule_id in BUILTIN_RULE_IDS:
        stats = merged.get(
            rule_id,
            {
                "total": 0,
                "tp": 0,
                "fp": 0,
                "unknown": 0,
                "examples": [],
                "unknown_buckets": Counter(),
                "fp_buckets": Counter(),
            },
        )
        tp = stats["tp"]
        fp = stats["fp"]
        unknown = stats["unknown"]
        total = stats["total"]
        passed = rule_passes(tp=tp, fp=fp, unknown=unknown, infrastructure=rule_id in INFRASTRUCTURE_RULES)
        rules[rule_id] = {
            "total": total,
            "tp": tp,
            "fp": fp,
            "unknown": unknown,
            "pass": passed,
            "pass_reason": pass_reason(
                tp=tp, fp=fp, unknown=unknown, total=total,
                infrastructure=rule_id in INFRASTRUCTURE_RULES,
            ),
            "infrastructure": rule_id in INFRASTRUCTURE_RULES,
            "unknown_buckets": dict(stats["unknown_buckets"]),
            "fp_buckets": dict(stats["fp_buckets"]),
            "examples": stats["examples"],
        }

    failing = [rid for rid, item in rules.items() if not item["pass"]]
    return {
        "repos": repos,
        "tp_target": TP_TARGET,
        "rules": rules,
        "summary": {
            "total_rules": len(BUILTIN_RULE_IDS),
            "passing": sum(1 for item in rules.values() if item["pass"]),
            "failing": len(failing),
            "failing_rules": failing,
        },
    }


def print_report(report: dict[str, Any]) -> None:
    summary = report["summary"]
    print(
        f"Census across {', '.join(report['repos'])}: "
        f"{summary['passing']}/{summary['total_rules']} rules PASS"
    )
    if summary["failing_rules"]:
        print(f"FAIL ({len(summary['failing_rules'])}): {', '.join(summary['failing_rules'])}")
    print()
    for rule_id, stats in report["rules"].items():
        if stats["pass"] and stats["fp"] == 0 and stats["unknown"] == 0:
            continue
        status = "PASS" if stats["pass"] else "FAIL"
        print(
            f"{status} {rule_id}: total={stats['total']} tp={stats['tp']} "
            f"fp={stats['fp']} unknown={stats['unknown']} ({stats['pass_reason']})"
        )
        if stats["fp_buckets"]:
            for key, count in sorted(stats["fp_buckets"].items(), key=lambda x: -x[1])[:5]:
                print(f"  fp bucket {key}: {count}")
        if stats["unknown_buckets"]:
            for key, count in sorted(stats["unknown_buckets"].items(), key=lambda x: -x[1])[:5]:
                print(f"  unknown bucket {key}: {count}")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--repos",
        nargs="+",
        default=DEFAULT_REPOS,
        help="benchmark repo names (default: all four)",
    )
    parser.add_argument(
        "--cache",
        type=Path,
        default=Path(
            os.environ.get(
                "COSTGUARD_BENCHMARK_CACHE",
                str(Path.home() / ".cache" / "costguard" / "benchmarks"),
            )
        ),
    )
    parser.add_argument("--force-compile", action="store_true")
    parser.add_argument("--json", action="store_true", help="emit JSON report")
    parser.add_argument(
        "--emit-evidence",
        action="store_true",
        help=f"write {EVIDENCE_PATH.relative_to(ROOT)}",
    )
    args = parser.parse_args()

    known = {repo["name"] for repo in load_repos()}
    for repo_name in args.repos:
        if repo_name not in known:
            raise SystemExit(f"unknown repo '{repo_name}'")

    merged: dict[str, dict[str, Any]] = {}
    for repo_name in args.repos:
        print(f"scanning {repo_name}...", file=sys.stderr)
        diagnostics, checkout, compiled = scan_repo(
            repo_name,
            cache_dir=args.cache,
            force_compile=args.force_compile,
        )
        repo_stats = aggregate_diagnostics(repo_name, diagnostics, checkout, compiled)
        merge_rule_stats(merged, repo_stats)
        print(f"  {len(diagnostics)} diagnostics", file=sys.stderr)

    report = build_report(merged, repos=args.repos)
    if args.emit_evidence:
        evidence = {
            rule_id: {
                "pass": stats["pass"],
                "pass_reason": stats["pass_reason"],
                "tp": stats["tp"],
                "fp": stats["fp"],
                "unknown": stats["unknown"],
                "total": stats["total"],
                "examples": stats["examples"],
            }
            for rule_id, stats in report["rules"].items()
        }
        EVIDENCE_PATH.parent.mkdir(parents=True, exist_ok=True)
        EVIDENCE_PATH.write_text(json.dumps(evidence, indent=2) + "\n", encoding="utf-8")
        print(f"wrote {EVIDENCE_PATH}", file=sys.stderr)

    if args.json:
        print(json.dumps(report, indent=2))
    else:
        print_report(report)

    return 0 if report["summary"]["failing"] == 0 else 1


if __name__ == "__main__":
    raise SystemExit(main())
