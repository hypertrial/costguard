#!/usr/bin/env python3
"""Unit tests for benchmark compare_report thresholds."""

from __future__ import annotations

import importlib.util
import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts" / "benchmark_external_repo.py"
sys.path.insert(0, str(ROOT / "scripts"))


def load_compare_report():
    spec = importlib.util.spec_from_file_location("benchmark_external_repo", SCRIPT)
    module = importlib.util.module_from_spec(spec)
    assert spec.loader is not None
    sys.modules["benchmark_external_repo"] = module
    spec.loader.exec_module(module)
    return module.compare_report


compare_report = load_compare_report()


class CompareReportTests(unittest.TestCase):
    def sample_report(self, rule_counts: dict[str, int]) -> dict:
        return {
            "metrics": {
                "diagnostics_by_rule": rule_counts,
                "sql_parse_failures": 0,
                "sql_parse_total": 1,
            }
        }

    def test_max_diagnostics_by_rule_passes_at_ceiling(self) -> None:
        report = self.sample_report({"SQLCOST012": 815})
        baseline = {"metrics": {"sql_parse_failures": 0}, "thresholds": {"max_diagnostics_by_rule": {"SQLCOST012": 815}}}
        self.assertEqual(compare_report(report, baseline), [])

    def test_max_diagnostics_by_rule_fails_above_ceiling(self) -> None:
        report = self.sample_report({"SQLCOST012": 816})
        baseline = {"metrics": {"sql_parse_failures": 0}, "thresholds": {"max_diagnostics_by_rule": {"SQLCOST012": 815}}}
        errors = compare_report(report, baseline)
        self.assertEqual(len(errors), 1)
        self.assertIn("SQLCOST012", errors[0])
        self.assertIn("> max 815", errors[0])

    def test_max_diagnostics_by_rule_allows_decrease(self) -> None:
        report = self.sample_report({"SQLCOST012": 500})
        baseline = {"metrics": {"sql_parse_failures": 0}, "thresholds": {"max_diagnostics_by_rule": {"SQLCOST012": 815}}}
        self.assertEqual(compare_report(report, baseline), [])

    def test_repeated_runtime_and_memory_gates(self) -> None:
        report = {
            "runtime_median_ms": 2000,
            "runtime_max_ms": 3000,
            "max_rss_bytes": 100,
            "metrics": {
                "sql_parse_failures": 0,
                "sql_parse_total": 1,
                "diagnostics_by_rule": {},
            },
        }
        baseline = {
            "metrics": {"sql_parse_failures": 0},
            "thresholds": {
                "max_runtime_median_ms": 15000,
                "max_runtime_max_ms": 20000,
                "max_rss_bytes": 1000,
            },
        }
        self.assertEqual(compare_report(report, baseline), [])
        report["runtime_median_ms"] = 16000
        report["runtime_max_ms"] = 21000
        report["max_rss_bytes"] = 1001
        errors = compare_report(report, baseline)
        self.assertEqual(len(errors), 3)
        self.assertTrue(any("runtime_median_ms" in error for error in errors))
        self.assertTrue(any("runtime_max_ms" in error for error in errors))
        self.assertTrue(any("max_rss_bytes" in error for error in errors))


if __name__ == "__main__":
    unittest.main()
