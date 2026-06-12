#!/usr/bin/env python3
from __future__ import annotations

import subprocess
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


class RecallReportTests(unittest.TestCase):
    def test_recall_gate_passes_for_default_rules(self) -> None:
        completed = subprocess.run(
            ["python3", "scripts/recall_report.py"],
            cwd=ROOT,
            capture_output=True,
            text=True,
            check=False,
        )
        self.assertEqual(
            completed.returncode,
            0,
            msg=completed.stdout + completed.stderr,
        )
        self.assertIn("Recall coverage gate passed", completed.stdout)


if __name__ == "__main__":
    unittest.main()
