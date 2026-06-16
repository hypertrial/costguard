#!/usr/bin/env python3
"""Compute binary-classification metrics for Costguard rule findings."""

from __future__ import annotations

import argparse
import json
import sys
from collections import defaultdict
from dataclasses import dataclass
from pathlib import Path
from typing import Any

import numpy as np
from sklearn.metrics import (
    average_precision_score,
    balanced_accuracy_score,
    confusion_matrix,
    f1_score,
    matthews_corrcoef,
    precision_score,
    recall_score,
    roc_auc_score,
)

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))

from bucket_rule_diagnostics import (  # noqa: E402
    CLASSIFIERS,
    load_manifest_sql,
    read_sql_for_diagnostic,
)
from costguard_tooling import repo_by_name, run_costguard_scan  # noqa: E402
from eval_lib import (  # noqa: E402
    DEFAULT_LABELS,
    EvalLabel,
    corpus_case_path,
    diagnostic_score,
    load_eval_labels,
    normalize_path,
    repo_checkout,
    wilson_ci,
)

CORPUS_GATES = {
    "precision_min": 1.0,
    "recall_min": 1.0,
    "mcc_min": 1.0,
}
REAL_GATES = {
    "precision_min": 0.80,
    "high_precision_min": 0.90,
    "per_rule_precision_min": 0.70,
}


@dataclass
class EvalRow:
    label: EvalLabel
    y_pred: int
    score: float
    severity: str | None = None


def scan_corpus_case(case_path: str, platform: str = "generic") -> dict[str, Any]:
    root = corpus_case_path(case_path)
    manifest = root / "target" / "manifest.json"
    payload, _ = run_costguard_scan(
        root,
        warehouse=platform,
        scan_paths=["models"],
        fail_on="critical",
        manifest=manifest if manifest.exists() else None,
        cost=True,
    )
    return payload


def corpus_platform(case_path: str) -> str:
    import tomllib

    from eval_lib import CORPUS_MANIFEST

    data = tomllib.loads(CORPUS_MANIFEST.read_text(encoding="utf-8"))
    for case in data.get("case", []):
        if case["path"] == case_path:
            return case.get("platform") or "generic"
    return "generic"


def rules_fired_in_case(payload: dict[str, Any]) -> dict[str, float]:
    scores: dict[str, float] = {}
    for diagnostic in payload.get("diagnostics", []):
        rule_id = diagnostic.get("rule_id")
        if not rule_id:
            continue
        score = diagnostic_score(diagnostic)
        scores[rule_id] = max(scores.get(rule_id, 0.0), score)
    return scores


def evaluate_corpus_labels(labels: list[EvalLabel]) -> list[EvalRow]:
    case_cache: dict[str, dict[str, float]] = {}
    rows: list[EvalRow] = []
    for label in labels:
        if label.y_true is None or label.pending:
            continue
        if label.path not in case_cache:
            payload = scan_corpus_case(label.path, corpus_platform(label.path))
            case_cache[label.path] = rules_fired_in_case(payload)
        fired = case_cache[label.path]
        y_pred = 1 if label.rule in fired else 0
        score = fired.get(label.rule, 0.0)
        rows.append(EvalRow(label=label, y_pred=y_pred, score=score))
    return rows


def scan_external_repo(repo_name: str, cache: Path) -> tuple[dict[str, Any], Path, dict[str, str]]:
    repo = repo_by_name(repo_name)
    checkout = repo_checkout(repo_name, cache)
    manifest = checkout / "target" / "manifest.json"
    if not manifest.exists():
        raise SystemExit(
            f"missing manifest at {manifest}; run benchmark_external_repo.py --repo {repo_name} first"
        )
    payload, _ = run_costguard_scan(
        checkout,
        warehouse=repo.get("warehouse", "generic"),
        scan_paths=repo.get("scan_paths", ["."]),
        fail_on="critical",
        manifest=manifest,
        cost=bool(repo.get("cost", False)),
    )
    compiled = load_manifest_sql(manifest)
    return payload, checkout, compiled


def bucket_verdict_map(labels: list[EvalLabel]) -> dict[tuple[str, str, str], EvalLabel]:
    mapping: dict[tuple[str, str, str], EvalLabel] = {}
    for label in labels:
        bucket = label.bucket()
        if bucket is None:
            continue
        key = (label.repo, label.rule, bucket)
        mapping[key] = label
    return mapping


def path_verdict_map(labels: list[EvalLabel]) -> dict[tuple[str, str, str], EvalLabel]:
    mapping: dict[tuple[str, str, str], EvalLabel] = {}
    for label in labels:
        if label.bucket() is not None:
            continue
        key = (label.repo, label.rule, normalize_path(label.path))
        mapping[key] = label
    return mapping


def evaluate_real_labels(labels: list[EvalLabel], cache: Path) -> list[EvalRow]:
    by_repo: dict[str, list[EvalLabel]] = defaultdict(list)
    for label in labels:
        if label.y_true is None or label.pending:
            continue
        by_repo[label.repo].append(label)

    rows: list[EvalRow] = []
    for repo_name, repo_labels in sorted(by_repo.items()):
        payload, checkout, compiled = scan_external_repo(repo_name, cache)
        bucket_map = bucket_verdict_map(repo_labels)
        path_map = path_verdict_map(repo_labels)

        matched_bucket_keys: set[tuple[str, str, str]] = set()
        matched_path_keys: set[tuple[str, str, str]] = set()

        for diagnostic in payload.get("diagnostics", []):
            rule_id = diagnostic.get("rule_id", "")
            rel_path = normalize_path(str(diagnostic.get("path", "")))
            sql = read_sql_for_diagnostic(checkout, diagnostic, compiled)
            classifier = CLASSIFIERS.get(rule_id, lambda _sql: "other")
            bucket = classifier(sql)
            severity = str(diagnostic.get("severity", "unknown")).lower()
            score = diagnostic_score(diagnostic)

            bucket_key = (repo_name, rule_id, bucket)
            bucket_label = bucket_map.get(bucket_key)
            if bucket_label is not None:
                rows.append(
                    EvalRow(
                        label=bucket_label,
                        y_pred=1,
                        score=score,
                        severity=severity,
                    )
                )
                matched_bucket_keys.add(bucket_key)

            path_key = (repo_name, rule_id, rel_path)
            path_label = path_map.get(path_key)
            if path_label is not None:
                rows.append(
                    EvalRow(
                        label=path_label,
                        y_pred=1,
                        score=score,
                        severity=severity,
                    )
                )
                matched_path_keys.add(path_key)

        for label in repo_labels:
            if label.bucket() is not None:
                key = (label.repo, label.rule, label.bucket() or "")
                if key not in matched_bucket_keys:
                    rows.append(EvalRow(label=label, y_pred=0, score=0.0))
            else:
                key = (label.repo, label.rule, normalize_path(label.path))
                if key not in matched_path_keys:
                    rows.append(EvalRow(label=label, y_pred=0, score=0.0))
    return rows


def rows_to_arrays(rows: list[EvalRow]) -> tuple[np.ndarray, np.ndarray, np.ndarray, np.ndarray]:
    if not rows:
        empty = np.array([], dtype=float)
        return empty, empty, empty, empty
    y_true = np.array([row.label.y_true for row in rows], dtype=int)
    y_pred = np.array([row.y_pred for row in rows], dtype=int)
    y_score = np.array([row.score for row in rows], dtype=float)
    weights = np.array([row.label.weight for row in rows], dtype=float)
    return y_true, y_pred, y_score, weights


def compute_metric_block(
    y_true: np.ndarray,
    y_pred: np.ndarray,
    y_score: np.ndarray,
    sample_weight: np.ndarray | None = None,
) -> dict[str, Any]:
    if y_true.size == 0:
        return {"n": 0}

    kwargs: dict[str, Any] = {}
    if sample_weight is not None and sample_weight.size:
        kwargs["sample_weight"] = sample_weight

    cm = confusion_matrix(y_true, y_pred, labels=[0, 1])
    tn, fp, fn, tp = cm.ravel()
    unique_true = len(np.unique(y_true))
    unique_pred = len(np.unique(y_pred))

    if unique_true < 2 or unique_pred < 2:
        precision = float(tp / (tp + fp)) if tp + fp else None
        recall = float(tp / (tp + fn)) if tp + fn else None
        if precision is not None and recall is not None and precision + recall:
            f1 = 2 * precision * recall / (precision + recall)
        else:
            f1 = None
        mcc = None
        balanced_acc = None
        pr_auc = None
        roc_auc = None
    else:
        precision = float(precision_score(y_true, y_pred, zero_division=0, **kwargs))
        recall = float(recall_score(y_true, y_pred, zero_division=0, **kwargs))
        f1 = float(f1_score(y_true, y_pred, zero_division=0, **kwargs))
        mcc = float(matthews_corrcoef(y_true, y_pred, sample_weight=sample_weight))
        balanced_acc = float(
            balanced_accuracy_score(y_true, y_pred, sample_weight=sample_weight)
        )
        if len(np.unique(y_score)) > 1:
            pr_auc = float(
                average_precision_score(y_true, y_score, sample_weight=sample_weight)
            )
            roc_auc = float(roc_auc_score(y_true, y_score, sample_weight=sample_weight))
        else:
            pr_auc = None
            roc_auc = None

    block: dict[str, Any] = {
        "n": int(y_true.size),
        "confusion_matrix": {"tp": int(tp), "fp": int(fp), "tn": int(tn), "fn": int(fn)},
        "precision": precision,
        "recall": recall,
        "f1": f1,
        "mcc": mcc,
        "balanced_accuracy": balanced_acc,
        "precision_ci": wilson_ci(float(tp), float(tp + fp)),
        "recall_ci": wilson_ci(float(tp), float(tp + fn)),
        "pr_auc": pr_auc,
        "roc_auc": roc_auc,
    }
    return block


def metrics_by_rule(rows: list[EvalRow]) -> dict[str, Any]:
    by_rule: dict[str, list[EvalRow]] = defaultdict(list)
    for row in rows:
        by_rule[row.label.rule].append(row)
    return {
        rule: compute_metric_block(*rows_to_arrays(rule_rows))
        for rule, rule_rows in sorted(by_rule.items())
    }


def metrics_by_severity(rows: list[EvalRow]) -> dict[str, Any]:
    by_severity: dict[str, list[EvalRow]] = defaultdict(list)
    for row in rows:
        if row.severity is None:
            continue
        by_severity[row.severity].append(row)
    return {
        severity: compute_metric_block(*rows_to_arrays(severity_rows))
        for severity, severity_rows in sorted(by_severity.items())
    }


def gate_report(
    split: str,
    overall: dict[str, Any],
    by_rule: dict[str, Any],
    rows: list[EvalRow],
) -> dict[str, bool]:
    if overall.get("n", 0) == 0:
        return {"overall": False}

    if split == "corpus":
        return {
            "precision": (overall.get("precision") or 0) >= CORPUS_GATES["precision_min"],
            "recall": (overall.get("recall") or 0) >= CORPUS_GATES["recall_min"],
            "mcc": (overall.get("mcc") or 0) >= CORPUS_GATES["mcc_min"],
        }

    high_rows = [
        row
        for row in rows
        if row.severity in {"high", "critical", "crit"}
    ]
    high_block = compute_metric_block(*rows_to_arrays(high_rows)) if high_rows else {"n": 0}
    per_rule_ok = all(
        block.get("precision") is None
        or block["precision"] >= REAL_GATES["per_rule_precision_min"]
        or block["confusion_matrix"]["tp"] + block["confusion_matrix"]["fp"] < 10
        for block in by_rule.values()
        if block.get("n", 0) > 0
    )
    high_precision = (
        high_block.get("n", 0) == 0
        or (high_block.get("precision") or 0) >= REAL_GATES["high_precision_min"]
    )
    return {
        "overall_precision": (overall.get("precision") or 0) >= REAL_GATES["precision_min"],
        "per_rule": per_rule_ok,
        "high_precision": high_precision,
    }


def build_report(rows: list[EvalRow], split: str) -> dict[str, Any]:
    y_true, y_pred, y_score, weights = rows_to_arrays(rows)
    overall = compute_metric_block(y_true, y_pred, y_score, weights)
    by_rule = metrics_by_rule(rows)
    by_severity = metrics_by_severity(rows)
    passes = gate_report(split, overall, by_rule, rows)
    return {
        "split": split,
        "rows_evaluated": len(rows),
        "overall": overall,
        "by_rule": by_rule,
        "by_severity": by_severity,
        "gates": REAL_GATES if split == "real" else CORPUS_GATES,
        "passes": passes,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--labels", type=Path, default=DEFAULT_LABELS)
    parser.add_argument(
        "--split",
        choices=("corpus", "real", "all"),
        default="corpus",
        help="Which label split to evaluate",
    )
    parser.add_argument(
        "--cache",
        type=Path,
        default=Path.home() / ".cache/costguard/benchmarks",
    )
    parser.add_argument("--json-out", type=Path, default=None)
    args = parser.parse_args()

    labels = load_eval_labels(args.labels)
    splits = ["corpus", "real"] if args.split == "all" else [args.split]
    reports: dict[str, Any] = {}
    exit_code = 0

    for split in splits:
        split_labels = [label for label in labels if label.split == split]
        if split == "corpus":
            rows = evaluate_corpus_labels(split_labels)
        else:
            rows = evaluate_real_labels(split_labels, args.cache)
        report = build_report(rows, split)
        reports[split] = report
        overall = report["overall"]
        print(f"Split {split}: {report['rows_evaluated']} rows evaluated")
        if overall.get("n", 0):
            print(
                f"  precision={overall['precision']:.3f} recall={overall['recall']:.3f} "
                f"f1={overall['f1']:.3f} mcc={overall['mcc']:.3f}"
            )
            pr_auc = overall.get("pr_auc")
            roc_auc = overall.get("roc_auc")
            if pr_auc is not None:
                print(f"  pr_auc={pr_auc:.3f} roc_auc={roc_auc:.3f}")
        for name, passed in report["passes"].items():
            status = "PASS" if passed else "FAIL"
            print(f"  gate {name}: {status}")
            if not passed:
                exit_code = 1

    if args.json_out is not None:
        args.json_out.parent.mkdir(parents=True, exist_ok=True)
        args.json_out.write_text(json.dumps(reports, indent=2) + "\n", encoding="utf-8")
        print(f"wrote {args.json_out}")

    return exit_code


if __name__ == "__main__":
    raise SystemExit(main())
