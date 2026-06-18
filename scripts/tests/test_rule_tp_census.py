#!/usr/bin/env python3
"""Tests for rule_tp_census.py pure helpers."""

from __future__ import annotations

import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))

from rule_tp_census import (  # noqa: E402
    adjudication_label,
    parse_failure_source_pattern,
    pass_reason,
    rule_passes,
    sample_findings,
    sample_stratified_tail,
    summarize_sample,
)


class RuleTpCensusTests(unittest.TestCase):
    def test_adjudication_label(self) -> None:
        self.assertEqual(adjudication_label(verdict="tp", fp_class=None), "tp")
        self.assertEqual(adjudication_label(verdict="fp", fp_class="exempt"), "exempt")
        self.assertEqual(adjudication_label(verdict="fp", fp_class="bug"), "fp_bug")
        self.assertEqual(adjudication_label(verdict="fp", fp_class=None), "fp_bug")
        self.assertEqual(adjudication_label(verdict=None, fp_class=None), "unknown")

    def test_cost_ranked_top_100(self) -> None:
        findings = [
            {"savings": 1.0, "repo": "a", "path": "x.sql", "line": 1, "label": "tp"},
            {"savings": 99.0, "repo": "a", "path": "y.sql", "line": 1, "label": "tp"},
            {"savings": 50.0, "repo": "b", "path": "z.sql", "line": 2, "label": "tp"},
        ]
        sampled = sample_findings(findings, sample_cap=2)
        self.assertEqual([item["savings"] for item in sampled], [99.0, 50.0])

    def test_sample_all_when_under_cap(self) -> None:
        findings = [
            {"savings": 1.0, "repo": "a", "path": "x.sql", "line": 1, "label": "tp"},
            {"savings": 2.0, "repo": "a", "path": "y.sql", "line": 1, "label": "exempt"},
        ]
        sampled = sample_findings(findings, sample_cap=100)
        self.assertEqual(len(sampled), 2)

    def test_stratified_tail_skips_primary_and_caps_per_repo_bucket(self) -> None:
        findings = [
            {
                "savings": 100.0,
                "repo": "a",
                "path": "primary.sql",
                "line": 1,
                "bucket": "x",
                "message": "m",
                "label": "tp",
            },
            {
                "savings": 50.0,
                "repo": "a",
                "path": "tail1.sql",
                "line": 2,
                "bucket": "x",
                "message": "m",
                "label": "tp",
            },
            {
                "savings": 40.0,
                "repo": "a",
                "path": "tail2.sql",
                "line": 3,
                "bucket": "x",
                "message": "m",
                "label": "tp",
            },
            {
                "savings": 30.0,
                "repo": "a",
                "path": "tail3.sql",
                "line": 4,
                "bucket": "y",
                "message": "m",
                "label": "tp",
            },
        ]
        sampled = sample_stratified_tail(findings, [findings[0]], per_bucket_cap=1)
        self.assertEqual([item["path"] for item in sampled], ["tail1.sql", "tail3.sql"])

    def test_parse_failure_source_patterns(self) -> None:
        self.assertEqual(
            parse_failure_source_pattern({"snippet": "{{ config(materialized='table') }}"}),
            "dbt_config_wrapper",
        )
        self.assertEqual(
            parse_failure_source_pattern({"snippet": "select * from x qualify rn = 1"}),
            "dialect_syntax",
        )

    def test_pass_when_no_fp_bug_or_unknown(self) -> None:
        findings = [
            {"label": "tp"},
            {"label": "exempt"},
        ]
        summary = summarize_sample(findings)
        self.assertTrue(rule_passes(fp_bug=summary["fp_bug"], unknown=summary["unknown"], infrastructure=False))
        self.assertEqual(
            pass_reason(
                tp=1,
                exempt=1,
                fp_bug=0,
                unknown=0,
                examined=2,
                total=2,
                infrastructure=False,
            ),
            "fully_examined",
        )

    def test_fail_on_unmarked_fp(self) -> None:
        summary = summarize_sample([{"label": "fp_bug"}])
        self.assertFalse(rule_passes(fp_bug=summary["fp_bug"], unknown=summary["unknown"], infrastructure=False))
        self.assertEqual(
            pass_reason(
                tp=0,
                exempt=0,
                fp_bug=1,
                unknown=0,
                examined=1,
                total=1,
                infrastructure=False,
            ),
            "fail",
        )

    def test_vacuous_clean(self) -> None:
        self.assertTrue(rule_passes(fp_bug=0, unknown=0, infrastructure=False))
        self.assertEqual(
            pass_reason(
                tp=0,
                exempt=0,
                fp_bug=0,
                unknown=0,
                examined=0,
                total=0,
                infrastructure=False,
            ),
            "vacuous_clean",
        )


if __name__ == "__main__":
    unittest.main()
