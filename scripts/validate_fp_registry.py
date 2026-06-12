#!/usr/bin/env python3
"""Validate fp_registry.toml entries against corpus expect/forbid contracts."""

from __future__ import annotations

import sys
import tomllib
from collections import defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
REGISTRY = ROOT / "tests" / "benchmarks" / "fp_registry.toml"
CORPUS_MANIFEST = ROOT / "tests" / "fixtures" / "corpus" / "manifest.toml"


def load_corpus_rules() -> dict[str, dict[str, set[str]]]:
    data = tomllib.loads(CORPUS_MANIFEST.read_text(encoding="utf-8"))
    by_case: dict[str, dict[str, set[str]]] = {}
    for case in data.get("case", []):
        name = case["name"]
        by_case[name] = {
            "expect": set(case.get("expect_rules", [])),
            "forbid": set(case.get("forbid_rules", [])),
        }
    return by_case


def main() -> int:
    if not REGISTRY.exists():
        print(f"missing registry: {REGISTRY}", file=sys.stderr)
        return 1

    registry = tomllib.loads(REGISTRY.read_text(encoding="utf-8"))
    corpus = load_corpus_rules()
    errors: list[str] = []
    repo_bucket_verdicts: dict[tuple[str, str, str], set[str]] = defaultdict(set)

    for idx, finding in enumerate(registry.get("finding", []), start=1):
        rule = finding.get("rule")
        bucket = finding.get("bucket")
        verdict = finding.get("verdict")
        repo = finding.get("repo", "spellbook")
        corpus_case = finding.get("corpus_case")

        if not rule or not bucket or not verdict:
            errors.append(f"finding #{idx}: requires rule, bucket, and verdict")
            continue

        repo_bucket_verdicts[(repo, rule, bucket)].add(verdict)

        if not corpus_case:
            errors.append(f"finding #{idx}: missing corpus_case")
            continue

        if corpus_case not in corpus:
            errors.append(
                f"finding #{idx}: corpus case '{corpus_case}' not found in manifest.toml"
            )
            continue

        if verdict == "fp":
            if rule not in corpus[corpus_case]["forbid"]:
                errors.append(
                    f"finding #{idx}: corpus case '{corpus_case}' must forbid {rule}, "
                    f"has expect={sorted(corpus[corpus_case]['expect']) or 'none'} "
                    f"forbid={sorted(corpus[corpus_case]['forbid']) or 'none'}"
                )
        elif verdict == "tp":
            if rule not in corpus[corpus_case]["expect"]:
                errors.append(
                    f"finding #{idx}: corpus case '{corpus_case}' must expect {rule}, "
                    f"has expect={sorted(corpus[corpus_case]['expect']) or 'none'} "
                    f"forbid={sorted(corpus[corpus_case]['forbid']) or 'none'}"
                )
        else:
            errors.append(f"finding #{idx}: unknown verdict '{verdict}'")

    for (repo, rule, bucket), verdicts in sorted(repo_bucket_verdicts.items()):
        if len(verdicts) > 1:
            errors.append(
                f"conflicting verdicts for {repo}/{rule}/{bucket}: {sorted(verdicts)}; "
                "split buckets or dedupe entries"
            )

    if errors:
        for error in errors:
            print(f"FAIL {error}", file=sys.stderr)
        return 1

    findings = registry.get("finding", [])
    fp_count = sum(1 for f in findings if f.get("verdict") == "fp")
    tp_count = sum(1 for f in findings if f.get("verdict") == "tp")
    print(
        f"fp registry valid ({fp_count} fp + {tp_count} tp entries checked against corpus)"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
