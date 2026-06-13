#!/usr/bin/env python3

from __future__ import annotations

import importlib.util
import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts" / "support_matrix.py"
SPEC = importlib.util.spec_from_file_location("support_matrix", SCRIPT)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = MODULE
SPEC.loader.exec_module(MODULE)


class SupportMatrixTest(unittest.TestCase):
    def test_repository_matrix_is_valid(self) -> None:
        self.assertEqual(MODULE.validate_matrix(MODULE.load_matrix()), [])

    def test_rejects_mutable_commit(self) -> None:
        repos = [
            {
                "name": "sample",
                "url": "https://github.com/acme/sample.git",
                "commit": "main",
                "warehouse": "generic",
                "scan_paths": ["."],
                "required": False,
            }
        ]
        self.assertTrue(any("full lowercase SHA" in error for error in MODULE.validate_matrix(repos)))


if __name__ == "__main__":
    unittest.main()
