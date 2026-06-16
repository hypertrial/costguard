#!/usr/bin/env python3
"""Tests for committed benchmark cost configs."""

from __future__ import annotations

import sys
import tempfile
import tomllib
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))

from costguard_tooling import (  # noqa: E402
    BENCHMARK_COST_CONFIGS,
    apply_benchmark_cost_config,
)

EXPECTED = {
    "data-infra": {"default_table_size": "medium", "usd_per_tb": 6.25},
    "spellbook": {"default_table_size": "medium", "usd_per_tb": 5.0},
}


class BenchmarkCostConfigTests(unittest.TestCase):
    def test_committed_configs_parse(self) -> None:
        for name, want in EXPECTED.items():
            path = BENCHMARK_COST_CONFIGS / f"{name}.toml"
            self.assertTrue(path.is_file(), msg=f"missing {path}")
            text = path.read_text(encoding="utf-8")
            self.assertIn("ESTIMATE ONLY", text)
            cfg = tomllib.loads(text)
            cost = cfg["cost"]
            self.assertEqual(cost["default_table_size"], want["default_table_size"])
            self.assertEqual(cost["pricing"]["usd_per_tb"], want["usd_per_tb"])

    def test_apply_writes_costguard_toml(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            checkout = Path(tmp)
            repo = {"name": "data-infra"}
            self.assertTrue(apply_benchmark_cost_config(checkout, repo))
            dst = checkout / "costguard.toml"
            self.assertTrue(dst.is_file())
            cfg = tomllib.loads(dst.read_text(encoding="utf-8"))
            self.assertEqual(cfg["cost"]["pricing"]["usd_per_tb"], 6.25)

    def test_apply_noop_when_missing(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            checkout = Path(tmp)
            repo = {"name": "jaffle-shop"}
            self.assertFalse(apply_benchmark_cost_config(checkout, repo))
            self.assertFalse((checkout / "costguard.toml").exists())


if __name__ == "__main__":
    unittest.main()
