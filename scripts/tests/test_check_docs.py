#!/usr/bin/env python3

from __future__ import annotations

import importlib.util
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts" / "check_docs.py"
SPEC = importlib.util.spec_from_file_location("check_docs", SCRIPT)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = MODULE
SPEC.loader.exec_module(MODULE)


class PublicVersionPinTest(unittest.TestCase):
    def check_text(self, text: str, current: str = "2.4.0") -> list[str]:
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "README.md"
            path.write_text(text, encoding="utf-8")
            return MODULE.check_public_version_pins([path], current)

    def test_current_public_pins_pass(self) -> None:
        errors = self.check_text(
            "\n".join(
                [
                    "- uses: hypertrial/costguard/.github/actions/costguard@v2.4.0",
                    "cargo install --git https://github.com/hypertrial/costguard --tag v2.4.0 costguard-cli",
                    "curl -fsSL https://example.invalid/install.sh | sh -s -- v2.4.0",
                    "VERSION=v2.4.0",
                    'COSTGUARD_VERSION: "v2.4.0"',
                    "COSTGUARD_VERSION = 'v2.4.0'",
                    "rev: v2.4.0",
                ]
            )
        )
        self.assertEqual(errors, [])

    def test_stale_public_pins_fail(self) -> None:
        errors = self.check_text(
            "\n".join(
                [
                    "- uses: hypertrial/costguard/.github/actions/costguard@v2.3.0",
                    "cargo install --git https://github.com/hypertrial/costguard --tag v2.3.0 costguard-cli",
                    "curl -fsSL https://example.invalid/install.sh | sh -s -- v2.3.0",
                    "VERSION=v2.3.0",
                    'COSTGUARD_VERSION: "v2.3.0"',
                    "COSTGUARD_VERSION = 'v2.3.0'",
                    "rev: v2.3.0",
                ]
            )
        )
        self.assertEqual(len(errors), 7)
        self.assertTrue(all("does not match workspace version v2.4.0" in error for error in errors))

    def test_historical_version_prose_is_ignored(self) -> None:
        errors = self.check_text(
            "`v2.3.0` added stale-manifest detection; `v2.2.0` added cost reporting."
        )
        self.assertEqual(errors, [])


if __name__ == "__main__":
    unittest.main()
