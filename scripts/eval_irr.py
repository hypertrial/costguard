#!/usr/bin/env python3
"""Validate committed LLM judge labels and compute inter-rater reliability."""

from __future__ import annotations

import argparse
import json
import sys
from collections import Counter, defaultdict
from pathlib import Path
from typing import Any

from sklearn.metrics import cohen_kappa_score

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))

from llm_judge_lib import (  # noqa: E402
    DEFAULT_IRR_REPORT,
    DEFAULT_LABELS_JSONL,
    DEFAULT_MANIFEST,
    JudgeManifest,
    JudgeRecord,
    cache_key,
    load_judge_records,
    load_manifest,
    utc_now_iso,
)

MIN_RULE_SAMPLES = 5
TOP_DISAGREEMENTS = 10


def validate_records(records: list[JudgeRecord], manifest: JudgeManifest) -> list[str]:
    errors: list[str] = []
    for record in records:
        if record.prompt_version != manifest.prompt_version:
            errors.append(
                f"{record.finding_id}: prompt_version mismatch "
                f"({record.prompt_version} != {manifest.prompt_version})"
            )
        if record.model_sha256 != manifest.model_file_sha256:
            errors.append(
                f"{record.finding_id}: model_sha256 mismatch "
                f"({record.model_sha256} != {manifest.model_file_sha256})"
            )
        expected = cache_key(
            finding_id=record.finding_id,
            rule_id=record.rule_id,
            rule_description_sha=record.rule_description_sha,
            sql_sha=record.sql_sha,
            finding_span=record.finding_span,
            prompt_version=record.prompt_version,
            model_file_sha256=record.model_sha256,
            runtime_version=record.runtime_version,
            mode=record.mode,
        )
        if record.cache_key != expected:
            errors.append(f"{record.finding_id}: cache_key mismatch")
    return errors


def kappa_or_none(y_true: list[str], y_pred: list[str]) -> float | None:
    if len(y_true) < 2:
        return None
    if len(set(y_true)) < 2 and len(set(y_pred)) < 2:
        return 1.0 if y_true == y_pred else None
    return float(cohen_kappa_score(y_true, y_pred, labels=["tp", "fp"]))


def build_report(records: list[JudgeRecord], manifest: JudgeManifest) -> dict[str, Any]:
    total = len(records)
    abstain = [record for record in records if record.llm_verdict == "unsure"]
    labeled = [record for record in records if record.registry_verdict in {"tp", "fp"}]
    scorable = [
        record
        for record in records
        if record.registry_verdict in {"tp", "fp"} and record.llm_verdict in {"tp", "fp"}
    ]

    registry_labels = [record.registry_verdict for record in scorable]
    llm_labels = [record.llm_verdict for record in scorable]
    disagreements = [
        {
            "finding_id": record.finding_id,
            "rule_id": record.rule_id,
            "bucket": record.bucket,
            "registry_verdict": record.registry_verdict,
            "llm_verdict": record.llm_verdict,
            "path": record.path,
            "line": record.line,
        }
        for record in scorable
        if record.registry_verdict != record.llm_verdict
    ]

    by_rule: dict[str, list[JudgeRecord]] = defaultdict(list)
    for record in scorable:
        by_rule[record.rule_id].append(record)

    per_rule: dict[str, Any] = {}
    for rule_id, rule_records in sorted(by_rule.items()):
        if len(rule_records) < MIN_RULE_SAMPLES:
            continue
        per_rule[rule_id] = {
            "n": len(rule_records),
            "kappa": kappa_or_none(
                [item.registry_verdict for item in rule_records],
                [item.llm_verdict for item in rule_records],
            ),
            "disagreements": sum(
                1
                for item in rule_records
                if item.registry_verdict != item.llm_verdict
            ),
        }

    rule_disagreement_counts = Counter(
        (record.rule_id, record.bucket)
        for record in scorable
        if record.registry_verdict != record.llm_verdict
    )
    top_disagreements = [
        {"rule_id": rule_id, "bucket": bucket, "count": count}
        for (rule_id, bucket), count in rule_disagreement_counts.most_common(
            TOP_DISAGREEMENTS
        )
    ]

    coverage = len(scorable) / total if total else 0.0
    abstain_rate = len(abstain) / total if total else 0.0
    disagreement_rate = (
        sum(1 for record in scorable if record.registry_verdict != record.llm_verdict)
        / len(scorable)
        if scorable
        else None
    )

    return {
        "generated_at": utc_now_iso(),
        "manifest": {
            "judge_name": manifest.judge_name,
            "judge_version": manifest.judge_version,
            "model_id": manifest.model_id,
            "prompt_version": manifest.prompt_version,
            "repo": manifest.repo,
        },
        "counts": {
            "total": total,
            "labeled_registry": len(labeled),
            "scorable_non_abstain": len(scorable),
            "abstain": len(abstain),
        },
        "overall": {
            "kappa_binary_non_abstain": kappa_or_none(registry_labels, llm_labels),
            "coverage": coverage,
            "abstain_rate": abstain_rate,
            "disagreement_rate": disagreement_rate,
        },
        "by_rule": per_rule,
        "top_disagreement_rules": top_disagreements,
        "disagreements": disagreements[:TOP_DISAGREEMENTS],
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--labels", type=Path, default=DEFAULT_LABELS_JSONL)
    parser.add_argument("--manifest", type=Path, default=DEFAULT_MANIFEST)
    parser.add_argument("--json-out", type=Path, default=DEFAULT_IRR_REPORT)
    args = parser.parse_args()

    manifest = load_manifest(args.manifest)
    records = load_judge_records(args.labels)
    errors = validate_records(records, manifest)
    if errors:
        for error in errors:
            print(f"validation error: {error}", file=sys.stderr)
        return 1

    report = build_report(records, manifest)
    args.json_out.parent.mkdir(parents=True, exist_ok=True)
    args.json_out.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")

    overall = report["overall"]
    counts = report["counts"]
    print(
        f"IRR: total={counts['total']} scorable={counts['scorable_non_abstain']} "
        f"kappa={overall['kappa_binary_non_abstain']} "
        f"coverage={overall['coverage']:.3f} abstain={overall['abstain_rate']:.3f}"
    )
    print(f"wrote {args.json_out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
