#!/usr/bin/env python3
"""Rank top cost findings for manual review (Spellbook or other external repos)."""

from __future__ import annotations

import argparse
import json
import os
import sys
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))

from bucket_rule_diagnostics import (  # noqa: E402
    load_manifest_sql,
    load_repo,
    read_sql_for_diagnostic,
)
from costguard_tooling import (  # noqa: E402
    apply_benchmark_cost_config,
    run_costguard_scan,
)
from eval_lib import repo_checkout  # noqa: E402
from precision_triage import classify_diagnostic  # noqa: E402


def savings_key(diagnostic: dict[str, Any]) -> float:
    cost = diagnostic.get("cost_estimate") or {}
    savings = cost.get("savings_p50_usd_per_month")
    if isinstance(savings, (int, float)):
        return float(savings)
    rel = cost.get("relative_index")
    if isinstance(rel, (int, float)):
        return float(rel)
    return 0.0


def sql_context(sql: str, line: int, *, radius: int) -> str:
    if not sql:
        return ""
    lines = sql.splitlines()
    if line <= 1:
        end = min(len(lines), radius * 2)
        return "\n".join(f"{idx + 1:4d}| {lines[idx]}" for idx in range(end))
    start = max(0, line - 1 - radius)
    end = min(len(lines), line + radius)
    out: list[str] = []
    for idx in range(start, end):
        prefix = ">>" if idx + 1 == line else "  "
        out.append(f"{prefix}{idx + 1:4d}| {lines[idx]}")
    return "\n".join(out)


def build_packets(
    repo_name: str,
    *,
    top: int,
    rule_filter: str | None,
    context: int,
) -> list[dict[str, Any]]:
    cache = Path(
        os.environ.get(
            "COSTGUARD_BENCHMARK_CACHE",
            str(Path.home() / ".cache" / "costguard" / "benchmarks"),
        )
    )
    repo = load_repo(repo_name)
    checkout = repo_checkout(repo_name, cache)
    manifest = checkout / "target" / "manifest.json"
    compiled = load_manifest_sql(manifest) if manifest.is_file() else {}
    apply_benchmark_cost_config(checkout, repo)

    payload, _ = run_costguard_scan(
        checkout,
        warehouse=repo.get("warehouse", "generic"),
        scan_paths=repo.get("scan_paths", ["."]),
        fail_on="critical",
        manifest=manifest if manifest.is_file() else None,
        cost=True,
    )

    with_cost = [d for d in payload.get("diagnostics", []) if d.get("cost_estimate")]
    if rule_filter:
        with_cost = [d for d in with_cost if d.get("rule_id") == rule_filter]
    ranked = sorted(with_cost, key=savings_key, reverse=True)[:top]

    packets: list[dict[str, Any]] = []
    for rank, diagnostic in enumerate(ranked, 1):
        sql = read_sql_for_diagnostic(checkout, diagnostic, compiled)
        bucket, registry = classify_diagnostic(diagnostic, checkout, compiled, repo_name)
        line = int(diagnostic.get("line") or 0)
        cost = diagnostic.get("cost_estimate") or {}
        packets.append(
            {
                "rank": rank,
                "rule_id": diagnostic.get("rule_id"),
                "path": diagnostic.get("path"),
                "line": line,
                "message": diagnostic.get("message"),
                "confidence": diagnostic.get("confidence"),
                "savings_p50_usd_per_month": cost.get("savings_p50_usd_per_month"),
                "relative_index": cost.get("relative_index"),
                "bucket": bucket,
                "registry_verdict": registry,
                "sql_context": sql_context(sql, line, radius=context),
            }
        )
    return packets


def print_packets(packets: list[dict[str, Any]]) -> None:
    for packet in packets:
        print(f"=== {packet['rank']}. {packet['rule_id']} {packet['path']}:{packet['line']} ===")
        print(f"message: {packet['message']}")
        print(
            f"confidence: {packet['confidence']} "
            f"savings_p50={packet['savings_p50_usd_per_month']} "
            f"rel_index={packet['relative_index']}"
        )
        print(f"bucket: {packet['bucket']} registry: {packet['registry_verdict']}")
        if packet["sql_context"]:
            print("--- SQL context ---")
            print(packet["sql_context"])
        print()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", default="spellbook", help="repo name from tests/benchmarks/repos.toml")
    parser.add_argument("--top", type=int, default=10, help="number of findings to rank")
    parser.add_argument("--rule", help="filter to a single rule_id")
    parser.add_argument("--context", type=int, default=12, help="SQL context lines around finding")
    parser.add_argument("--json", action="store_true", help="emit JSON instead of text")
    args = parser.parse_args()

    packets = build_packets(
        args.repo,
        top=args.top,
        rule_filter=args.rule,
        context=args.context,
    )
    if args.json:
        print(json.dumps(packets, indent=2))
    else:
        print_packets(packets)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
