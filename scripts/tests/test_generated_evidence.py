from __future__ import annotations

import importlib.util
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


def load_script(name: str):
    path = ROOT / "scripts" / f"{name}.py"
    spec = importlib.util.spec_from_file_location(name, path)
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class GeneratedEvidenceTests(unittest.TestCase):
    def test_every_registry_rule_receives_a_tier(self) -> None:
        generator = load_script("generate_precision_tiers")
        rule_ids = sorted(generator.INFRASTRUCTURE_RULES | {"SQLCOST001"})
        tiers = generator.build_tiers({}, rule_ids)
        self.assertEqual(set(tiers), set(rule_ids))
        self.assertEqual(tiers["SQLCOST046"]["tier"], "informational")

    def test_orphaned_infrastructure_rule_fails_generation(self) -> None:
        generator = load_script("generate_precision_tiers")
        with self.assertRaisesRegex(ValueError, "SQLCOST046"):
            generator.build_tiers({}, ["SQLCOST001"])

    def test_precision_generation_is_byte_identical(self) -> None:
        generator = load_script("generate_precision_tiers")
        rule_ids = sorted(generator.INFRASTRUCTURE_RULES | {"SQLCOST001"})
        evidence = {"SQLCOST001": {"pass": True, "examined": 10, "tp": 9}}
        first = generator.build_tiers(evidence, rule_ids)
        second = generator.build_tiers(evidence, rule_ids)
        self.assertEqual(
            generator.render_toml(first, rule_ids),
            generator.render_toml(second, rule_ids),
        )
        self.assertEqual(
            generator.render_rust(first, rule_ids),
            generator.render_rust(second, rule_ids),
        )

    def test_benchmark_snapshot_has_no_wall_clock_fields(self) -> None:
        builder = load_script("build_benchmark_evidence")
        rules = {"SQLCOST046": {"severity": "high"}}
        tiers = {"SQLCOST046": {"tier": "informational"}}
        snapshot = builder.build_snapshot({}, tiers, rules)
        self.assertNotIn("generated_at", snapshot)
        self.assertNotIn("generated ", builder.render_markdown(snapshot))
