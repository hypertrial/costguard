#!/usr/bin/env python3
"""Validate fp_registry.toml entries against corpus forbid_rules contracts."""

from __future__ import annotations

import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
REGISTRY = ROOT / "tests" / "benchmarks" / "fp_registry.toml"
CORPUS_MANIFEST = ROOT / "tests" / "fixtures" / "corpus" / "manifest.toml"

try:
    import tomllib
except ModuleNotFoundError:
    import tomli as tomllib  # type: ignore[no-redef]


def load_corpus_forbid_rules() -> dict[str, set[str]]:
    data = tomllib.loads(CORPUS_MANIFEST.read_text(encoding="utf-8"))
    by_case: dict[str, set[str]] = {}
    for case in data.get("case", []):
        name = case["name"]
        by_case[name] = set(case.get("forbid_rules", []))
    return by_case


def main() -> int:
    if not REGISTRY.exists():
        print(f"missing registry: {REGISTRY}", file=sys.stderr)
        return 1

    registry = tomllib.loads(REGISTRY.read_text(encoding="utf-8"))
    corpus = load_corpus_forbid_rules()
    errors: list[str] = []

    for idx, finding in enumerate(registry.get("finding", []), start=1):
        verdict = finding.get("verdict")
        if verdict != "fp":
            continue

        rule = finding.get("rule")
        corpus_case = finding.get("corpus_case")
        if not rule or not corpus_case:
            errors.append(f"finding #{idx}: fp verdict requires rule and corpus_case")
            continue

        if corpus_case not in corpus:
            errors.append(
                f"finding #{idx}: corpus case '{corpus_case}' not found in manifest.toml"
            )
            continue

        if rule not in corpus[corpus_case]:
            errors.append(
                f"finding #{idx}: corpus case '{corpus_case}' must forbid {rule}, "
                f"has {sorted(corpus[corpus_case]) or 'none'}"
            )

    if errors:
        for error in errors:
            print(f"FAIL {error}", file=sys.stderr)
        return 1

    fp_count = sum(1 for f in registry.get("finding", []) if f.get("verdict") == "fp")
    print(f"fp registry valid ({fp_count} fp entries checked against corpus)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
