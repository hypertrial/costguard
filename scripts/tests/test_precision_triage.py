#!/usr/bin/env python3
"""Tests for precision_triage.py."""

from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
import sys

sys.path.insert(0, str(ROOT / "scripts"))

from precision_triage import precision_report, sample_diagnostics  # noqa: E402


class PrecisionTriageTests(unittest.TestCase):
    def test_sample_includes_all_high_findings(self) -> None:
        diagnostics = [
            {"rule_id": "SQLCOST012", "severity": "high", "path": "a.sql", "line": 1},
            {"rule_id": "SQLCOST006", "severity": "med", "path": "b.sql", "line": 1},
            {"rule_id": "SQLCOST008", "severity": "low", "path": "c.sql", "line": 1},
        ]
        sampled = sample_diagnostics(diagnostics, sample_size=2, seed=1)
        self.assertEqual(len(sampled), 2)
        self.assertEqual(sampled[0]["severity"], "high")

    def test_precision_report_counts_registry_verdicts(self) -> None:
        diagnostics = [
            {
                "rule_id": "SQLCOST012",
                "severity": "high",
                "path": "models/a.sql",
                "line": 1,
                "message": "cross join",
            }
        ]
        checkout = ROOT / "tests/fixtures/real_world/spellbook_snippets"
        report = precision_report(diagnostics, checkout, {})
        self.assertEqual(report["sample_size"], 1)
        self.assertIn("passes", report)


if __name__ == "__main__":
    unittest.main()
