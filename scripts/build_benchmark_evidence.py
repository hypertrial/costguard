#!/usr/bin/env python3
"""Build committed benchmark evidence snapshot and mdBook page."""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
import tomllib
from collections import Counter
from datetime import date
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))

EVIDENCE = ROOT / "tests" / "benchmarks" / "rule_tp_evidence.json"
TIERS = ROOT / "tests" / "benchmarks" / "precision_tiers.toml"
REPOS = ROOT / "tests" / "benchmarks" / "repos.toml"
FP_REGISTRY = ROOT / "tests" / "benchmarks" / "fp_registry.toml"
WORKSPACE_VERSION = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))[
    "workspace"
]["package"]["version"]
RELEASE_LINE = ".".join(WORKSPACE_VERSION.split(".")[:2])
SNAPSHOT_OUT = ROOT / "tests" / "benchmarks" / "evidence" / f"v{RELEASE_LINE}.json"
DOC_OUT = ROOT / "docs" / "book" / "reference" / "benchmarks.md"
GENERATED_START = "<!-- generated:evidence:start -->"
GENERATED_END = "<!-- generated:evidence:end -->"

INFRASTRUCTURE_RULES = {
    "SQLCOST023",
    "SQLCOST024",
    "SQLCOST025",
    "SQLCOST026",
    "SQLCOST027",
    "SQLCOST045",
    "SQLCOST046",
}
CENSUS_EXCLUDED_RULES = {"SQLCOST045", "SQLCOST046"}
ENTERPRISE_GATES = {
    "high_severity_sampled_precision_min": 0.90,
    "overall_sampled_precision_min": 0.80,
    "per_rule_sampled_precision_min": 0.70,
}


def fetch_rule_metadata() -> dict[str, dict[str, str]]:
    from costguard_tooling import costguard_binary

    proc = subprocess.run(
        [str(costguard_binary()), "rules", "--format", "json"],
        cwd=ROOT,
        check=False,
        capture_output=True,
        text=True,
    )
    if proc.returncode != 0:
        raise SystemExit(proc.stderr.strip() or "costguard rules failed")
    return {rule["id"]: rule for rule in json.loads(proc.stdout)}


def load_tiers() -> dict[str, dict[str, str]]:
    data = tomllib.loads(TIERS.read_text(encoding="utf-8"))
    return {entry["id"]: entry for entry in data.get("rule", [])}


def sampled_precision(entry: dict) -> float | None:
    examined = int(entry.get("examined") or 0)
    if examined == 0:
        return None
    tp = int(entry.get("tp") or 0)
    return tp / examined


def build_snapshot(
    evidence: dict,
    tiers: dict[str, dict[str, str]],
    rules: dict[str, dict[str, str]],
) -> dict:
    behavioral: list[tuple[str, dict]] = []
    high_behavioral: list[tuple[str, dict]] = []
    per_rule = []
    tier_counts: Counter[str] = Counter()
    examples = []

    for rule_id, meta in sorted(rules.items()):
        entry = evidence.get(rule_id, {})
        tier = tiers.get(rule_id, {}).get("tier", "unverified")
        tier_counts[tier] += 1
        precision = sampled_precision(entry) if entry else None
        severity = meta.get("severity", "unknown")
        per_rule.append(
            {
                "rule_id": rule_id,
                "severity": severity,
                "tier": tier,
                "pass": entry.get("pass"),
                "sampled_precision": precision,
                "tp": entry.get("tp"),
                "examined": entry.get("examined"),
                "total": entry.get("total"),
            }
        )
        if rule_id in INFRASTRUCTURE_RULES:
            continue
        if entry.get("pass") and int(entry.get("examined") or 0) > 0:
            behavioral.append((rule_id, entry))
            if severity == "high":
                high_behavioral.append((rule_id, entry))
        for sample in (entry.get("examined_examples") or [])[:2]:
            examples.append({"rule_id": rule_id, **sample})

    def aggregate_precision(rows: list[tuple[str, dict]]) -> float | None:
        tp = sum(int(entry.get("tp") or 0) for _, entry in rows)
        examined = sum(int(entry.get("examined") or 0) for _, entry in rows)
        return (tp / examined) if examined else None

    overall = aggregate_precision(behavioral)
    high_overall = aggregate_precision(high_behavioral)
    behavioral_rules = [rule_id for rule_id in rules if rule_id not in CENSUS_EXCLUDED_RULES]
    passing = sum(1 for rule_id in behavioral_rules if evidence.get(rule_id, {}).get("pass"))

    repos = tomllib.loads(REPOS.read_text(encoding="utf-8")).get("repo", [])
    registry = tomllib.loads(FP_REGISTRY.read_text(encoding="utf-8")).get("finding", [])
    fp_entries = [finding for finding in registry if finding.get("verdict") == "fp"]
    bug_entries = [finding for finding in fp_entries if finding.get("class") == "bug"]

    return {
        "version": WORKSPACE_VERSION,
        "generated_at": date.today().isoformat(),
        "quality_ledger": {
            "evidence_snapshot": str(SNAPSHOT_OUT.relative_to(ROOT)),
            "full_repositories": [repo["name"] for repo in repos],
            "smoke_repositories": [
                repo["name"]
                for repo in repos
                if repo.get("smoke_scan_paths") or repo.get("smoke_compile_dirs")
            ],
            "false_positive_registry": {
                "status": "clean" if not bug_entries else "open-bugs",
                "entries": len(registry),
                "false_positives": len(fp_entries),
                "documented_exemptions": sum(
                    finding.get("class") == "exempt" for finding in fp_entries
                ),
                "open_bugs": len(bug_entries),
            },
        },
        "enterprise_gates": ENTERPRISE_GATES,
        "headline": {
            "overall_sampled_precision": overall,
            "high_severity_sampled_precision": high_overall,
            "rules_passing_census": passing,
            "rules_total": len(behavioral_rules),
            "tier_counts": dict(sorted(tier_counts.items())),
        },
        "per_rule": per_rule,
        "examples": examples[:12],
    }


def render_markdown(snapshot: dict) -> str:
    headline = snapshot["headline"]
    gates = snapshot["enterprise_gates"]
    overall = headline.get("overall_sampled_precision")
    high = headline.get("high_severity_sampled_precision")
    overall_pct = f"{overall * 100:.1f}%" if overall is not None else "n/a"
    high_pct = f"{high * 100:.1f}%" if high is not None else "n/a"
    ledger = snapshot["quality_ledger"]
    fp = ledger["false_positive_registry"]

    tier_lines = "\n".join(
        f"- **{tier}:** {count}"
        for tier, count in sorted(headline.get("tier_counts", {}).items())
    )

    example_lines = []
    for item in snapshot.get("examples", []):
        example_lines.append(
            f"- `{item['rule_id']}` {item.get('repo', '')} "
            f"`{item.get('path', '')}:{item.get('line', '')}` — {item.get('message', '')}"
        )
    examples_block = "\n".join(example_lines) if example_lines else "- (none)"

    generated = f"""## Quality ledger

| Evidence | Current value |
| --- | --- |
| Release version | `{snapshot['version']}` |
| Evidence snapshot | `{ledger['evidence_snapshot']}` |
| Full benchmark repositories | {len(ledger['full_repositories'])}: {', '.join(ledger['full_repositories'])} |
| Smoke benchmark repositories | {len(ledger['smoke_repositories'])}: {', '.join(ledger['smoke_repositories'])} |
| False-positive registry | {fp['status']}; {fp['entries']} verdicts, {fp['documented_exemptions']} documented exemptions, {fp['open_bugs']} open bugs |

## Headline metrics

| Metric | Value | Enterprise gate |
| --- | --- | --- |
| Overall sampled precision | {overall_pct} | ≥ {gates['overall_sampled_precision_min'] * 100:.0f}% |
| High-severity sampled precision | {high_pct} | ≥ {gates['high_severity_sampled_precision_min'] * 100:.0f}% |
| Per-rule sampled precision | see tier table | ≥ {gates['per_rule_sampled_precision_min'] * 100:.0f}% each classified rule |
| Rules passing TP census | {headline['rules_passing_census']}/{headline['rules_total']} | 44/44 behavioral |

## Precision tiers

{tier_lines}

## Example true positives (real dbt repos)

{examples_block}

Regenerate this page:

```bash
python3 scripts/build_benchmark_evidence.py
```
"""

    return "\n".join(
        [
            "# Benchmark evidence",
            "",
            "Public snapshot of Costguard precision/recall evidence from real dbt benchmark repos "
            "and the corpus regression suite.",
            "",
            f"Snapshot: [`tests/benchmarks/evidence/v{RELEASE_LINE}.json`](../../../tests/benchmarks/evidence/v{RELEASE_LINE}.json) "
            f"(generated {snapshot['generated_at']}).",
            "",
            GENERATED_START,
            generated.rstrip(),
            GENERATED_END,
            "",
        ]
    )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--check",
        action="store_true",
        help="Exit 1 when snapshot or benchmarks.md is stale",
    )
    args = parser.parse_args()

    if not EVIDENCE.exists():
        print(f"missing evidence: {EVIDENCE}", file=sys.stderr)
        return 1
    for path in (TIERS, REPOS, FP_REGISTRY):
        if not path.exists():
            print(f"missing benchmark input: {path}", file=sys.stderr)
            return 1

    evidence = json.loads(EVIDENCE.read_text(encoding="utf-8"))
    tiers = load_tiers()
    rules = fetch_rule_metadata()
    snapshot = build_snapshot(evidence, tiers, rules)
    snapshot_text = json.dumps(snapshot, indent=2, sort_keys=True) + "\n"
    doc_text = render_markdown(snapshot)

    if args.check:
        stale = False
        for path, expected in ((SNAPSHOT_OUT, snapshot_text), (DOC_OUT, doc_text)):
            if not path.exists() or path.read_text(encoding="utf-8") != expected:
                print(f"stale benchmark evidence: {path}", file=sys.stderr)
                stale = True
        return 1 if stale else 0

    SNAPSHOT_OUT.parent.mkdir(parents=True, exist_ok=True)
    SNAPSHOT_OUT.write_text(snapshot_text, encoding="utf-8")
    DOC_OUT.write_text(doc_text, encoding="utf-8")
    print(f"wrote {SNAPSHOT_OUT}")
    print(f"wrote {DOC_OUT}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
