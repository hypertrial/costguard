#!/usr/bin/env python3
"""Tests for eval_lib.py, build_eval_dataset.py, and eval_metrics.py."""

from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

try:
    import numpy as np
except ImportError:  # pragma: no cover - CI uses .venv-eval
    np = None

ROOT = Path(__file__).resolve().parents[2]
import sys

sys.path.insert(0, str(ROOT / "scripts"))

from build_eval_dataset import (  # noqa: E402
    merge_labels,
    seed_corpus_labels,
    seed_fp_registry_labels,
)
from eval_lib import (  # noqa: E402
    EvalLabel,
    diagnostic_score,
    load_eval_labels,
    wilson_ci,
    write_eval_labels,
)

try:
    from eval_metrics import compute_metric_block, rows_to_arrays  # noqa: E402
except ImportError:  # pragma: no cover - CI uses .venv-eval
    compute_metric_block = None
    rows_to_arrays = None

EVAL_DEPS = np is not None and compute_metric_block is not None


class EvalLibTests(unittest.TestCase):
    def test_wilson_ci_bounds(self) -> None:
        low, high = wilson_ci(8, 10)
        self.assertIsNotNone(low)
        self.assertIsNotNone(high)
        assert low is not None and high is not None
        self.assertLess(low, 0.9)
        self.assertGreater(high, 0.7)

    def test_wilson_ci_empty(self) -> None:
        self.assertEqual(wilson_ci(0, 0), (None, None))

    def test_diagnostic_score_prefers_cost(self) -> None:
        diagnostic = {
            "severity": "high",
            "confidence": "high",
            "cost_estimate": {"current_cost_p50_usd_per_month": 42.0},
        }
        self.assertEqual(diagnostic_score(diagnostic), 42.0)

    def test_diagnostic_score_fallback(self) -> None:
        diagnostic = {"severity": "high", "confidence": "medium"}
        self.assertEqual(diagnostic_score(diagnostic), 6.0)

    def test_label_round_trip(self) -> None:
        labels = [
            EvalLabel(
                repo="corpus",
                sha="fixtures",
                rule="SQLCOST005",
                path="incremental_missing",
                y_true=1,
                split="corpus",
                source="seed:corpus",
            )
        ]
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "labels.toml"
            write_eval_labels(labels, path)
            loaded = load_eval_labels(path)
        self.assertEqual(len(loaded), 1)
        self.assertEqual(loaded[0].rule, "SQLCOST005")
        self.assertEqual(loaded[0].y_true, 1)


class DatasetBuilderTests(unittest.TestCase):
    def test_corpus_seed_has_expect_and_forbid(self) -> None:
        labels = seed_corpus_labels()
        positives = [label for label in labels if label.y_true == 1]
        negatives = [label for label in labels if label.y_true == 0]
        self.assertGreater(len(positives), 0)
        self.assertGreater(len(negatives), 0)
        self.assertTrue(all(label.split == "corpus" for label in labels))

    def test_fp_registry_seed_uses_bucket_paths(self) -> None:
        labels = seed_fp_registry_labels()
        self.assertGreater(len(labels), 0)
        self.assertTrue(all(label.path.startswith("__bucket__:") for label in labels))

    def test_merge_labels_dedupes(self) -> None:
        left = [
            EvalLabel(
                repo="corpus",
                sha="fixtures",
                rule="SQLCOST005",
                path="case_a",
                y_true=1,
                split="corpus",
                source="seed:corpus",
            )
        ]
        right = [
            EvalLabel(
                repo="corpus",
                sha="fixtures",
                rule="SQLCOST005",
                path="case_a",
                y_true=0,
                split="corpus",
                source="seed:corpus",
            )
        ]
        merged = merge_labels(left, right)
        self.assertEqual(len(merged), 1)
        self.assertEqual(merged[0].y_true, 0)


class EvalMetricsTests(unittest.TestCase):
    @unittest.skipIf(not EVAL_DEPS, "eval deps not installed (pip install --require-hashes -r requirements-eval.lock)")
    def test_perfect_confusion_matrix(self) -> None:
        y_true = np.array([1, 1, 0, 0])
        y_pred = np.array([1, 1, 0, 0])
        y_score = np.array([10.0, 8.0, 0.0, 0.0])
        block = compute_metric_block(y_true, y_pred, y_score)
        self.assertEqual(block["confusion_matrix"]["tp"], 2)
        self.assertEqual(block["confusion_matrix"]["tn"], 2)
        self.assertEqual(block["precision"], 1.0)
        self.assertEqual(block["recall"], 1.0)
        self.assertEqual(block["mcc"], 1.0)

    @unittest.skipIf(not EVAL_DEPS, "eval deps not installed (pip install --require-hashes -r requirements-eval.lock)")
    def test_weighted_rows_to_arrays(self) -> None:
        from eval_metrics import EvalRow

        rows = [
            EvalRow(
                label=EvalLabel(
                    repo="corpus",
                    sha="fixtures",
                    rule="SQLCOST005",
                    path="a",
                    y_true=1,
                    split="corpus",
                    source="seed:corpus",
                    weight=2.0,
                ),
                y_pred=1,
                score=5.0,
            )
        ]
        _, _, _, weights = rows_to_arrays(rows)
        self.assertEqual(weights[0], 2.0)


class FrozenLabelsTests(unittest.TestCase):
    def test_repo_eval_labels_loads(self) -> None:
        labels = load_eval_labels(ROOT / "tests" / "benchmarks" / "eval_labels.toml")
        self.assertGreaterEqual(len(labels), 100)
        splits = {label.split for label in labels}
        self.assertIn("corpus", splits)
        self.assertIn("real", splits)


if __name__ == "__main__":
    unittest.main()
