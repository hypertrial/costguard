#!/usr/bin/env python3
"""Shared helpers for Costguard binary-classification evaluation."""

from __future__ import annotations

import math
import tomllib
from dataclasses import dataclass
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[1]
DEFAULT_LABELS = ROOT / "tests" / "benchmarks" / "eval_labels.toml"
CORPUS_ROOT = ROOT / "tests" / "fixtures" / "corpus"
CORPUS_MANIFEST = CORPUS_ROOT / "manifest.toml"
FP_REGISTRY = ROOT / "tests" / "benchmarks" / "fp_registry.toml"
REPOS_TOML = ROOT / "tests" / "benchmarks" / "repos.toml"

BUCKET_PATH_PREFIX = "__bucket__:"
CORPUS_REPO = "corpus"
CORPUS_SHA = "fixtures"

SEVERITY_RANK = {"info": 1, "low": 2, "med": 2, "medium": 2, "high": 3, "critical": 4, "crit": 4}
CONFIDENCE_RANK = {"low": 1, "medium": 2, "med": 2, "high": 3}


@dataclass(frozen=True)
class EvalLabel:
    repo: str
    sha: str
    rule: str
    path: str
    y_true: int | None
    split: str
    source: str
    weight: float = 1.0
    notes: str = ""
    pending: bool = False

    def bucket(self) -> str | None:
        if self.path.startswith(BUCKET_PATH_PREFIX):
            return self.path.removeprefix(BUCKET_PATH_PREFIX)
        return None

    def is_corpus_case(self) -> bool:
        return self.repo == CORPUS_REPO and not self.path.startswith(BUCKET_PATH_PREFIX)


def load_eval_labels(path: Path | None = None) -> list[EvalLabel]:
    labels_path = path or DEFAULT_LABELS
    data = tomllib.loads(labels_path.read_text(encoding="utf-8"))
    labels: list[EvalLabel] = []
    for row in data.get("label", []):
        y_raw = row.get("y_true")
        y_true = None if y_raw is None else int(y_raw)
        labels.append(
            EvalLabel(
                repo=str(row["repo"]),
                sha=str(row["sha"]),
                rule=str(row["rule"]),
                path=str(row["path"]),
                y_true=y_true,
                split=str(row.get("split", "corpus")),
                source=str(row.get("source", "unknown")),
                weight=float(row.get("weight", 1.0)),
                notes=str(row.get("notes", "")),
                pending=bool(row.get("pending", False)),
            )
        )
    return labels


def write_eval_labels(labels: list[EvalLabel], path: Path, *, version: int = 1) -> None:
    lines = [
        "# Frozen binary-classification labels for Costguard rule evaluation.",
        "# Regenerate with: python3 scripts/build_eval_dataset.py --write",
        "",
        "[meta]",
        f'version = {version}',
        'generated_by = "build_eval_dataset.py"',
        "",
    ]
    for label in labels:
        lines.extend(
            [
                "[[label]]",
                f'repo = "{label.repo}"',
                f'sha = "{label.sha}"',
                f'rule = "{label.rule}"',
                f'path = "{label.path}"',
            ]
        )
        if label.y_true is None:
            lines.append("y_true = null")
        else:
            lines.append(f"y_true = {label.y_true}")
        lines.extend(
            [
                f'split = "{label.split}"',
                f'source = "{label.source}"',
                f"weight = {label.weight}",
            ]
        )
        if label.pending:
            lines.append("pending = true")
        if label.notes:
            escaped = label.notes.replace('"', '\\"')
            lines.append(f'notes = "{escaped}"')
        lines.append("")
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text("\n".join(lines), encoding="utf-8")


def load_repos() -> dict[str, dict[str, Any]]:
    data = tomllib.loads(REPOS_TOML.read_text(encoding="utf-8"))
    return {repo["name"]: repo for repo in data.get("repo", [])}


def corpus_case_path(case_path: str) -> Path:
    return CORPUS_ROOT / case_path


def repo_checkout(repo_name: str, cache: Path) -> Path:
    return cache / repo_name


def wilson_ci(successes: float, total: float, z: float = 1.96) -> tuple[float | None, float | None]:
    if total <= 0:
        return None, None
    p_hat = successes / total
    denom = 1 + z**2 / total
    center = (p_hat + z**2 / (2 * total)) / denom
    margin = (
        z
        * math.sqrt((p_hat * (1 - p_hat) + z**2 / (4 * total)) / total)
        / denom
    )
    return max(0.0, center - margin), min(1.0, center + margin)


def severity_confidence_score(diagnostic: dict[str, Any]) -> float:
    severity = str(diagnostic.get("severity", "high")).lower()
    confidence = str(diagnostic.get("confidence", "high")).lower()
    return float(SEVERITY_RANK.get(severity, 2) * CONFIDENCE_RANK.get(confidence, 2))


def diagnostic_score(diagnostic: dict[str, Any]) -> float:
    cost = diagnostic.get("cost_estimate") or {}
    for key in (
        "current_cost_p50_usd_per_month",
        "savings_p50_usd_per_month",
        "model_monthly_p50_usd",
    ):
        value = cost.get(key)
        if isinstance(value, int | float) and value > 0:
            return float(value)
    return severity_confidence_score(diagnostic)


def normalize_path(path: str) -> str:
    return path.replace("\\", "/")


def label_sort_key(label: EvalLabel) -> tuple[str, ...]:
    return (label.split, label.repo, label.sha, label.rule, label.path)
