#!/usr/bin/env python3

from __future__ import annotations

import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))

from verify_ci_history import successful_runs  # noqa: E402


class VerifyCiHistoryTest(unittest.TestCase):
    def test_requires_three_latest_matching_runs_to_succeed(self) -> None:
        payload = {
            "workflow_runs": [
                {"head_sha": "release", "conclusion": "success", "id": 3},
                {"head_sha": "other", "conclusion": "failure", "id": 9},
                {"head_sha": "release", "conclusion": "success", "id": 2},
                {"head_sha": "release", "conclusion": "success", "id": 1},
            ]
        }
        self.assertEqual(len(successful_runs(payload, "release", 3)), 3)

    def test_rejects_failure_or_insufficient_history(self) -> None:
        with self.assertRaises(SystemExit):
            successful_runs({"workflow_runs": [{"head_sha": "x", "conclusion": "failure"}]}, "x", 1)
        with self.assertRaises(SystemExit):
            successful_runs({"workflow_runs": []}, "x", 3)


if __name__ == "__main__":
    unittest.main()
