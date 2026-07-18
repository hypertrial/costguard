#!/usr/bin/env python3
"""Validate committed LLM judge labels and compute inter-rater reliability."""

from __future__ import annotations

import argparse
import json
import os
import sys
import tempfile
from collections import Counter, defaultdict
from pathlib import Path
from typing import Any

from sklearn.metrics import cohen_kappa_score, matthews_corrcoef

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
            fewshots_sha=record.fewshots_sha,
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


def mcc_or_none(y_true: list[str], y_pred: list[str]) -> float | None:
    if len(y_true) < 2:
        return None
    label_to_int = {"fp": 0, "tp": 1}
    y_true_i = [label_to_int[item] for item in y_true]
    y_pred_i = [label_to_int[item] for item in y_pred]
    if len(set(y_true_i)) < 2 and len(set(y_pred_i)) < 2:
        return 1.0 if y_true_i == y_pred_i else None
    return float(matthews_corrcoef(y_true_i, y_pred_i))


def class_metrics(records: list[JudgeRecord]) -> dict[str, float | None]:
    registry_fp = [record for record in records if record.registry_verdict == "fp"]
    registry_tp = [record for record in records if record.registry_verdict == "tp"]
    judge_fp = [record for record in records if record.llm_verdict == "fp"]
    judge_tp = [record for record in records if record.llm_verdict == "tp"]

    def recall(registry_label: str, judge_label: str) -> float | None:
        subset = [record for record in records if record.registry_verdict == registry_label]
        if not subset:
            return None
        hits = sum(1 for record in subset if record.llm_verdict == judge_label)
        return hits / len(subset)

    def precision(judge_label: str, registry_label: str) -> float | None:
        subset = [record for record in records if record.llm_verdict == judge_label]
        if not subset:
            return None
        hits = sum(1 for record in subset if record.registry_verdict == registry_label)
        return hits / len(subset)

    return {
        "registry_fp_recall": recall("fp", "fp"),
        "registry_tp_recall": recall("tp", "tp"),
        "registry_fp_precision": precision("fp", "fp"),
        "registry_tp_precision": precision("tp", "tp"),
        "registry_fp_n": len(registry_fp),
        "registry_tp_n": len(registry_tp),
        "judge_fp_n": len(judge_fp),
        "judge_tp_n": len(judge_tp),
    }


def metrics_block(records: list[JudgeRecord]) -> dict[str, float | None]:
    if not records:
        return {
            "kappa_binary_non_abstain": None,
            "mcc": None,
            "registry_fp_recall": None,
            "registry_tp_recall": None,
            "registry_fp_precision": None,
            "registry_tp_precision": None,
        }
    registry_labels = [record.registry_verdict for record in records]
    llm_labels = [record.llm_verdict for record in records]
    block = {
        "kappa_binary_non_abstain": kappa_or_none(registry_labels, llm_labels),
        "mcc": mcc_or_none(registry_labels, llm_labels),
    }
    block.update(class_metrics(records))
    return block


def build_report(records: list[JudgeRecord], manifest: JudgeManifest) -> dict[str, Any]:
    total = len(records)
    abstain = [record for record in records if record.llm_verdict == "unsure"]
    labeled = [record for record in records if record.registry_verdict in {"tp", "fp"}]
    scorable = [
        record
        for record in records
        if record.registry_verdict in {"tp", "fp"} and record.llm_verdict in {"tp", "fp"}
    ]

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
        rule_metrics = metrics_block(rule_records)
        per_rule[rule_id] = {
            "n": len(rule_records),
            "kappa": rule_metrics["kappa_binary_non_abstain"],
            "mcc": rule_metrics["mcc"],
            "registry_fp_recall": rule_metrics["registry_fp_recall"],
            "registry_tp_recall": rule_metrics["registry_tp_recall"],
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

    overall_metrics = metrics_block(scorable)
    return {
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
            **overall_metrics,
            "coverage": coverage,
            "abstain_rate": abstain_rate,
            "disagreement_rate": disagreement_rate,
        },
        "by_rule": per_rule,
        "top_disagreement_rules": top_disagreements,
        "disagreements": disagreements[:TOP_DISAGREEMENTS],
    }


def render_report(report: dict[str, Any]) -> bytes:
    return (json.dumps(report, indent=2) + "\n").encode("utf-8")


def emit_report(path: Path, content: bytes, *, check: bool) -> bool:
    if check:
        try:
            return path.read_bytes() == content
        except FileNotFoundError:
            return False

    path.parent.mkdir(parents=True, exist_ok=True)
    temporary_path: Path | None = None
    try:
        with tempfile.NamedTemporaryFile(
            dir=path.parent,
            prefix=f".{path.name}.",
            mode="wb",
            delete=False,
        ) as temporary:
            temporary.write(content)
            temporary.flush()
            os.fsync(temporary.fileno())
            temporary_path = Path(temporary.name)
        os.replace(temporary_path, path)
        temporary_path = None
    finally:
        if temporary_path is not None:
            temporary_path.unlink(missing_ok=True)
    return True


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--labels", type=Path, default=DEFAULT_LABELS_JSONL)
    parser.add_argument("--manifest", type=Path, default=DEFAULT_MANIFEST)
    parser.add_argument("--json-out", type=Path, default=DEFAULT_IRR_REPORT)
    parser.add_argument(
        "--check",
        action="store_true",
        help="verify that the committed report is current without writing it",
    )
    args = parser.parse_args()

    manifest = load_manifest(args.manifest)
    records = load_judge_records(args.labels)
    errors = validate_records(records, manifest)
    if errors:
        for error in errors:
            print(f"validation error: {error}", file=sys.stderr)
        return 1

    report = build_report(records, manifest)
    if not emit_report(args.json_out, render_report(report), check=args.check):
        print(
            f"stale or missing IRR report: {args.json_out}; "
            "run scripts/eval_irr.py to refresh it",
            file=sys.stderr,
        )
        return 1

    overall = report["overall"]
    counts = report["counts"]
    print(
        f"IRR: total={counts['total']} scorable={counts['scorable_non_abstain']} "
        f"kappa={overall['kappa_binary_non_abstain']} mcc={overall['mcc']} "
        f"fp_recall={overall['registry_fp_recall']} "
        f"coverage={overall['coverage']:.3f} abstain={overall['abstain_rate']:.3f}"
    )
    print(f"{'verified' if args.check else 'wrote'} {args.json_out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
