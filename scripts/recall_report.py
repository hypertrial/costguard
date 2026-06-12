#!/usr/bin/env python3
"""Report corpus recall coverage and gate on minimum positive/negative contracts."""

from __future__ import annotations

import argparse
import json
import sys
from collections import defaultdict
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[1]
CORPUS_MANIFEST = ROOT / "tests" / "fixtures" / "corpus" / "manifest.toml"

try:
    import tomllib
except ModuleNotFoundError:
    import tomli as tomllib  # type: ignore[no-redef]

BEHAVIORAL_RULES = [f"SQLCOST{idx:03d}" for idx in range(1, 23)] + [
    f"SQLCOST{idx:03d}" for idx in range(28, 36)
]
MIN_EXPECT_CASES = 2
MIN_FORBID_CASES = 1


def load_manifest() -> list[dict[str, Any]]:
    data = tomllib.loads(CORPUS_MANIFEST.read_text(encoding="utf-8"))
    return data.get("case", [])


def coverage_by_rule(cases: list[dict[str, Any]]) -> dict[str, dict[str, set[str]]]:
    coverage: dict[str, dict[str, set[str]]] = defaultdict(
        lambda: {"expect": set(), "forbid": set()}
    )
    for case in cases:
        name = case["name"]
        for rule in case.get("expect_rules", []):
            coverage[rule]["expect"].add(name)
        for rule in case.get("forbid_rules", []):
            coverage[rule]["forbid"].add(name)
    return coverage


def build_report(
    rules: list[str],
    coverage: dict[str, dict[str, set[str]]],
) -> dict[str, Any]:
    by_rule: dict[str, Any] = {}
    errors: list[str] = []
    for rule in rules:
        expect_cases = sorted(coverage.get(rule, {}).get("expect", set()))
        forbid_cases = sorted(coverage.get(rule, {}).get("forbid", set()))
        by_rule[rule] = {
            "expect_cases": expect_cases,
            "forbid_cases": forbid_cases,
            "expect_count": len(expect_cases),
            "forbid_count": len(forbid_cases),
        }
        if len(expect_cases) < MIN_EXPECT_CASES:
            errors.append(
                f"{rule}: need >= {MIN_EXPECT_CASES} expect_rules cases, have {len(expect_cases)}"
            )
        if len(forbid_cases) < MIN_FORBID_CASES:
            errors.append(
                f"{rule}: need >= {MIN_FORBID_CASES} forbid_rules cases, have {len(forbid_cases)}"
            )
    return {
        "rules": by_rule,
        "min_expect_cases": MIN_EXPECT_CASES,
        "min_forbid_cases": MIN_FORBID_CASES,
        "passes": not errors,
        "errors": errors,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--rules",
        nargs="*",
        default=BEHAVIORAL_RULES,
        help="Rule ids to check (default: SQLCOST001-022)",
    )
    parser.add_argument("--json-out", type=Path, default=None)
    args = parser.parse_args()

    cases = load_manifest()
    coverage = coverage_by_rule(cases)
    report = build_report(args.rules, coverage)

    print(
        f"Recall coverage gate: {len(args.rules)} rules, "
        f">= {MIN_EXPECT_CASES} expect and >= {MIN_FORBID_CASES} forbid each"
    )
    for rule in args.rules:
        stats = report["rules"][rule]
        print(
            f"  {rule}: expect={stats['expect_count']} forbid={stats['forbid_count']}"
        )

    if report["errors"]:
        for error in report["errors"]:
            print(f"FAIL {error}", file=sys.stderr)
    else:
        print("Recall coverage gate passed")

    if args.json_out is not None:
        args.json_out.parent.mkdir(parents=True, exist_ok=True)
        args.json_out.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
        print(f"wrote {args.json_out}")

    return 0 if report["passes"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
