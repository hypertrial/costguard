#!/usr/bin/env python3
"""Sample Spellbook (or other repo) findings and compute precision metrics."""

from __future__ import annotations

import argparse
import json
import random
import sys
import tomllib
from collections import Counter, defaultdict
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))

from bucket_rule_diagnostics import (  # noqa: E402
    CLASSIFIERS,
    load_manifest_sql,
    load_repo,
    read_sql_for_diagnostic,
    run_scan,
)

FP_REGISTRY = ROOT / "tests" / "benchmarks" / "fp_registry.toml"
PRECISION_GATES = {
    "high": 0.90,
    "overall": 0.80,
    "per_rule_min": 0.70,
}
INFRASTRUCTURE_RULES = {
    "SQLCOST023",
    "SQLCOST024",
    "SQLCOST025",
    "SQLCOST026",
    "SQLCOST027",
}


def load_fp_registry() -> list[dict[str, Any]]:
    data = tomllib.loads(FP_REGISTRY.read_text(encoding="utf-8"))
    return data.get("finding", [])


def registry_entry_map() -> dict[tuple[str, str, str], dict[str, Any]]:
    entries: dict[tuple[str, str, str], dict[str, Any]] = {}
    for entry in load_fp_registry():
        rule = entry.get("rule")
        bucket = entry.get("bucket")
        verdict = entry.get("verdict")
        repo = entry.get("repo", "spellbook")
        if not rule or not bucket or not verdict:
            continue
        key = (repo, rule, bucket)
        if key in entries and entries[key].get("verdict") != verdict:
            raise SystemExit(
                f"conflicting fp_registry verdicts for {repo}/{rule}/{bucket}: "
                f"{entries[key].get('verdict')} vs {verdict}"
            )
        entries[key] = entry
    return entries


def registry_verdict_map() -> dict[tuple[str, str, str], str]:
    return {
        key: entry["verdict"]
        for key, entry in registry_entry_map().items()
        if entry.get("verdict")
    }


def registry_verdict(repo: str, rule: str, bucket: str) -> str | None:
    entries = registry_entry_map()
    entry = entries.get((repo, rule, bucket)) or entries.get(("spellbook", rule, bucket))
    if entry is None:
        return None
    return entry.get("verdict")


def registry_class(repo: str, rule: str, bucket: str) -> str | None:
    entries = registry_entry_map()
    entry = entries.get((repo, rule, bucket)) or entries.get(("spellbook", rule, bucket))
    if entry is None:
        return None
    return entry.get("class")


def classify_diagnostic(
    diagnostic: dict[str, Any],
    checkout: Path,
    compiled_by_path: dict[str, str],
    repo: str,
) -> tuple[str, str | None]:
    rule_id = diagnostic.get("rule_id", "")
    sql = read_sql_for_diagnostic(checkout, diagnostic, compiled_by_path)
    classifier = CLASSIFIERS.get(rule_id, lambda _sql: "other")
    bucket = classifier(sql)
    verdict = registry_verdict(repo, rule_id, bucket)
    return bucket, verdict


def sample_diagnostics(
    diagnostics: list[dict[str, Any]],
    *,
    sample_size: int,
    seed: int,
) -> list[dict[str, Any]]:
    highs = [d for d in diagnostics if d.get("severity") in {"high", "crit", "critical"}]
    others = [d for d in diagnostics if d not in highs]
    rng = random.Random(seed)
    selected = list(highs)
    remaining = max(0, sample_size - len(selected))
    if remaining and others:
        selected.extend(rng.sample(others, min(remaining, len(others))))
    return selected


def precision_report(
    diagnostics: list[dict[str, Any]],
    checkout: Path,
    compiled_by_path: dict[str, str],
    repo: str,
) -> dict[str, Any]:
    by_rule: dict[str, Counter[str]] = defaultdict(Counter)
    by_severity: Counter[str] = Counter()
    unknown_buckets: Counter[str] = Counter()

    for diagnostic in diagnostics:
        bucket, verdict = classify_diagnostic(diagnostic, checkout, compiled_by_path, repo)
        rule_id = diagnostic.get("rule_id", "unknown")
        severity = str(diagnostic.get("severity", "unknown")).lower()
        if verdict is None:
            unknown_buckets[f"{rule_id}:{bucket}"] += 1
            continue
        by_rule[rule_id][verdict] += 1
        if severity in {"high", "crit", "critical"}:
            by_severity[verdict] += 1

    def ratio(counter: Counter[str]) -> float | None:
        total = counter.get("tp", 0) + counter.get("fp", 0)
        if total == 0:
            return None
        return counter.get("tp", 0) / total

    rule_precision = {
        rule: {
            "tp": counts.get("tp", 0),
            "fp": counts.get("fp", 0),
            "precision": ratio(counts),
        }
        for rule, counts in sorted(by_rule.items())
    }
    high_tp = by_severity.get("tp", 0)
    high_fp = by_severity.get("fp", 0)
    high_total = high_tp + high_fp
    overall_tp = sum(item.get("tp", 0) for item in rule_precision.values())
    overall_fp = sum(item.get("fp", 0) for item in rule_precision.values())
    overall_total = overall_tp + overall_fp

    return {
        "sample_size": len(diagnostics),
        "classified": sum(sum(c.values()) for c in by_rule.values()),
        "unknown_buckets": dict(unknown_buckets),
        "high_precision": (high_tp / high_total) if high_total else None,
        "overall_precision": (overall_tp / overall_total) if overall_total else None,
        "by_rule": rule_precision,
        "gates": {
            "high_min": PRECISION_GATES["high"],
            "overall_min": PRECISION_GATES["overall"],
            "per_rule_min": PRECISION_GATES["per_rule_min"],
        },
        "passes": {
            "high": (high_tp / high_total) >= PRECISION_GATES["high"] if high_total else False,
            "overall": (overall_tp / overall_total) >= PRECISION_GATES["overall"]
            if overall_total
            else False,
            "per_rule": all(
                item["precision"] is None
                or item["precision"] >= PRECISION_GATES["per_rule_min"]
                or (item.get("tp", 0) + item.get("fp", 0)) < 10
                for item in rule_precision.values()
            ),
        },
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--scan-json",
        type=Path,
        help="Costguard JSON scan output (defaults to latest external spellbook report)",
    )
    parser.add_argument(
        "--checkout",
        type=Path,
        help="Repo checkout used for the scan",
    )
    parser.add_argument("--repo", default="spellbook", help="External repo name")
    parser.add_argument("--sample-size", type=int, default=200)
    parser.add_argument("--seed", type=int, default=42)
    parser.add_argument("--json-out", type=Path, default=None)
    args = parser.parse_args()

    scan_json = args.scan_json
    payload: dict[str, Any]
    checkout = args.checkout
    if scan_json is not None:
        payload = json.loads(scan_json.read_text(encoding="utf-8"))
    else:
        repo = load_repo(args.repo)
        if checkout is None:
            checkout = Path.home() / ".cache/costguard/benchmarks" / args.repo
        manifest = checkout / "target/manifest.json"
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
    diagnostics = [
        diagnostic
        for diagnostic in payload.get("diagnostics", [])
        if diagnostic.get("rule_id") not in INFRASTRUCTURE_RULES
    ]
    manifest = checkout / "target/manifest.json"
    if not manifest.exists():
        raise SystemExit(f"missing manifest at {manifest}; run benchmark first")
    compiled_by_path = load_manifest_sql(manifest)

    sampled = sample_diagnostics(diagnostics, sample_size=args.sample_size, seed=args.seed)
    report = precision_report(sampled, checkout, compiled_by_path, args.repo)

    print(f"Sampled {report['sample_size']} diagnostics ({report['classified']} classified)")
    if report["high_precision"] is not None:
        print(f"High-severity precision: {report['high_precision']:.3f}")
    if report["overall_precision"] is not None:
        print(f"Overall precision: {report['overall_precision']:.3f}")
    for rule, stats in report["by_rule"].items():
        precision = stats["precision"]
        label = f"{precision:.3f}" if precision is not None else "n/a"
        print(f"  {rule}: tp={stats['tp']} fp={stats['fp']} precision={label}")
    if report["unknown_buckets"]:
        print("Unknown buckets (extend fp_registry.toml):")
        for key, count in sorted(report["unknown_buckets"].items(), key=lambda item: -item[1]):
            print(f"  {key}: {count}")

    if args.json_out is not None:
        args.json_out.parent.mkdir(parents=True, exist_ok=True)
        args.json_out.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
        print(f"wrote {args.json_out}")

    return 0 if all(report["passes"].values()) else 1


if __name__ == "__main__":
    raise SystemExit(main())
