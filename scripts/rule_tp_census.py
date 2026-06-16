#!/usr/bin/env python3
"""Full-corpus per-rule FP-elimination census across benchmark repos."""

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
from precision_triage import classify_diagnostic, registry_class  # noqa: E402

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
SAMPLE_TARGET = 100


def snippet_for(sql: str, line: int, *, width: int = 240) -> str:
    if not sql:
        return ""
    lines = sql.splitlines()
    idx = max(0, min(len(lines) - 1, line - 1))
    text = lines[idx] if lines else ""
    return text.strip()[:width]


def savings_key(diagnostic: dict[str, Any]) -> float:
    cost = diagnostic.get("cost_estimate") or {}
    savings = cost.get("savings_p50_usd_per_month")
    if isinstance(savings, (int, float)):
        return float(savings)
    rel = cost.get("relative_index")
    if isinstance(rel, (int, float)):
        return float(rel)
    return 0.0


def adjudication_label(*, verdict: str | None, fp_class: str | None) -> str:
    if verdict is None:
        return "unknown"
    if verdict == "tp":
        return "tp"
    if verdict == "fp":
        if fp_class == "exempt":
            return "exempt"
        return "fp_bug"
    return "unknown"


def pass_reason(
    *,
    tp: int,
    exempt: int,
    fp_bug: int,
    unknown: int,
    examined: int,
    total: int,
    infrastructure: bool,
) -> str:
    if infrastructure:
        return "infrastructure_na"
    if total == 0:
        return "vacuous_clean"
    if fp_bug == 0 and unknown == 0:
        if examined < total:
            return f"sampled_{examined}_of_{total}"
        return "fully_examined"
    return "fail"


def rule_passes(*, fp_bug: int, unknown: int, infrastructure: bool) -> bool:
    if infrastructure:
        return True
    return fp_bug == 0 and unknown == 0


def sample_findings(
    findings: list[dict[str, Any]],
    *,
    sample_cap: int,
) -> list[dict[str, Any]]:
    if not findings:
        return []
    ranked = sorted(
        findings,
        key=lambda item: (
            -item["savings"],
            item.get("repo", ""),
            item.get("path", ""),
            item.get("line", 0),
        ),
    )
    return ranked[: min(sample_cap, len(ranked))]


def summarize_sample(findings: list[dict[str, Any]]) -> dict[str, Any]:
    counts = Counter(item["label"] for item in findings)
    return {
        "examined": len(findings),
        "tp": counts.get("tp", 0),
        "exempt": counts.get("exempt", 0),
        "fp_bug": counts.get("fp_bug", 0),
        "unknown": counts.get("unknown", 0),
    }


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
    payload, _ = run_costguard_scan(
        checkout,
        warehouse=repo.get("warehouse", "generic"),
        scan_paths=repo.get("scan_paths", ["."]),
        fail_on=repo.get("fail_on", "critical"),
        manifest=manifest if manifest.is_file() else None,
        cost=True,
    )
    return payload.get("diagnostics", []), checkout, compiled


def aggregate_diagnostics(
    repo_name: str,
    diagnostics: list[dict[str, Any]],
    checkout: Path,
    compiled: dict[str, str],
    *,
    rule_filter: str | None = None,
) -> dict[str, dict[str, Any]]:
    by_rule: dict[str, dict[str, Any]] = defaultdict(
        lambda: {
            "total": 0,
            "findings": [],
            "fp_bug_buckets": Counter(),
            "unknown_buckets": Counter(),
        }
    )
    for diagnostic in diagnostics:
        rule_id = diagnostic.get("rule_id", "")
        if not rule_id or (rule_filter and rule_id != rule_filter):
            continue
        bucket, verdict = classify_diagnostic(diagnostic, checkout, compiled, repo_name)
        fp_class = registry_class(repo_name, rule_id, bucket) if verdict == "fp" else None
        label = adjudication_label(verdict=verdict, fp_class=fp_class)
        line = int(diagnostic.get("line") or 0)
        sql = read_sql_for_diagnostic(checkout, diagnostic, compiled)
        entry = by_rule[rule_id]
        entry["total"] += 1
        entry["findings"].append(
            {
                "repo": repo_name,
                "path": diagnostic.get("path", ""),
                "line": line,
                "bucket": bucket,
                "verdict": verdict,
                "class": fp_class,
                "label": label,
                "message": diagnostic.get("message", ""),
                "savings": savings_key(diagnostic),
                "snippet": snippet_for(sql, line),
            }
        )
        if label == "fp_bug":
            entry["fp_bug_buckets"][f"{repo_name}:{bucket}"] += 1
        elif label == "unknown":
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
                "findings": [],
                "fp_bug_buckets": Counter(),
                "unknown_buckets": Counter(),
            },
        )
        entry["total"] += stats["total"]
        entry["findings"].extend(stats["findings"])
        entry["fp_bug_buckets"].update(stats["fp_bug_buckets"])
        entry["unknown_buckets"].update(stats["unknown_buckets"])


def build_report(
    merged: dict[str, dict[str, Any]],
    *,
    repos: list[str],
    sample_cap: int,
    rule_filter: str | None = None,
) -> dict[str, Any]:
    rule_ids = [rule_filter] if rule_filter else BUILTIN_RULE_IDS
    rules: dict[str, dict[str, Any]] = {}
    for rule_id in rule_ids:
        stats = merged.get(
            rule_id,
            {
                "total": 0,
                "findings": [],
                "fp_bug_buckets": Counter(),
                "unknown_buckets": Counter(),
            },
        )
        sampled = sample_findings(stats["findings"], sample_cap=sample_cap)
        summary = summarize_sample(sampled)
        passed = rule_passes(
            fp_bug=summary["fp_bug"],
            unknown=summary["unknown"],
            infrastructure=rule_id in INFRASTRUCTURE_RULES,
        )
        rules[rule_id] = {
            "total": stats["total"],
            "examined": summary["examined"],
            "tp": summary["tp"],
            "exempt": summary["exempt"],
            "fp_bug": summary["fp_bug"],
            "unknown": summary["unknown"],
            "pass": passed,
            "pass_reason": pass_reason(
                tp=summary["tp"],
                exempt=summary["exempt"],
                fp_bug=summary["fp_bug"],
                unknown=summary["unknown"],
                examined=summary["examined"],
                total=stats["total"],
                infrastructure=rule_id in INFRASTRUCTURE_RULES,
            ),
            "infrastructure": rule_id in INFRASTRUCTURE_RULES,
            "fp_bug_buckets": dict(stats["fp_bug_buckets"]),
            "unknown_buckets": dict(stats["unknown_buckets"]),
            "examined_examples": sampled,
        }

    failing = [rid for rid, item in rules.items() if not item["pass"]]
    return {
        "repos": repos,
        "sample_cap": sample_cap,
        "rules": rules,
        "summary": {
            "total_rules": len(rule_ids),
            "passing": sum(1 for item in rules.values() if item["pass"]),
            "failing": len(failing),
            "failing_rules": failing,
        },
    }


def print_report(report: dict[str, Any]) -> None:
    summary = report["summary"]
    print(
        f"Census across {', '.join(report['repos'])}: "
        f"{summary['passing']}/{summary['total_rules']} rules PASS "
        f"(sample cap {report['sample_cap']})"
    )
    if summary["failing_rules"]:
        print(f"FAIL ({len(summary['failing_rules'])}): {', '.join(summary['failing_rules'])}")
    print()
    for rule_id, stats in report["rules"].items():
        if stats["pass"] and stats["fp_bug"] == 0 and stats["unknown"] == 0:
            if stats["total"] == 0 or (
                stats["examined"] == stats["total"] and stats["exempt"] == 0
            ):
                continue
        status = "PASS" if stats["pass"] else "FAIL"
        print(
            f"{status} {rule_id}: total={stats['total']} examined={stats['examined']} "
            f"tp={stats['tp']} exempt={stats['exempt']} fp_bug={stats['fp_bug']} "
            f"unknown={stats['unknown']} ({stats['pass_reason']})"
        )
        if stats["fp_bug_buckets"]:
            for key, count in sorted(stats["fp_bug_buckets"].items(), key=lambda x: -x[1])[:5]:
                print(f"  fp_bug bucket {key}: {count}")
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
    parser.add_argument("--rule", help="single rule id to census (e.g. SQLCOST012)")
    parser.add_argument(
        "--sample-cap",
        type=int,
        default=SAMPLE_TARGET,
        help=f"max findings examined per rule (default: {SAMPLE_TARGET})",
    )
    parser.add_argument("--json", action="store_true", help="emit JSON report")
    parser.add_argument(
        "--emit-evidence",
        action="store_true",
        help=f"write {EVIDENCE_PATH.relative_to(ROOT)}",
    )
    args = parser.parse_args()

    if args.rule and args.rule not in BUILTIN_RULE_IDS:
        raise SystemExit(f"unknown rule '{args.rule}'")

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
        repo_stats = aggregate_diagnostics(
            repo_name,
            diagnostics,
            checkout,
            compiled,
            rule_filter=args.rule,
        )
        merge_rule_stats(merged, repo_stats)
        print(f"  {len(diagnostics)} diagnostics", file=sys.stderr)

    report = build_report(
        merged,
        repos=args.repos,
        sample_cap=args.sample_cap,
        rule_filter=args.rule,
    )
    if args.emit_evidence:
        evidence = {
            rule_id: {
                "pass": stats["pass"],
                "pass_reason": stats["pass_reason"],
                "total": stats["total"],
                "examined": stats["examined"],
                "tp": stats["tp"],
                "exempt": stats["exempt"],
                "fp_bug": stats["fp_bug"],
                "unknown": stats["unknown"],
                "examined_examples": stats["examined_examples"],
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
