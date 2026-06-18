#!/usr/bin/env python3

from __future__ import annotations

import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))

from verify_ci_history import qualifying_runs  # noqa: E402


class VerifyCiHistoryTest(unittest.TestCase):
    def setUp(self) -> None:
        self.jobs = {
            run_id: {
                "jobs": [
                    {"name": name, "status": "completed", "conclusion": "success"}
                    for name in ["pr-gate", "scale", "spellbook-smoke", "nba-monte-carlo-smoke"]
                ]
            }
            for run_id in [1, 2]
        }

    def test_accepts_one_successful_push_run(self) -> None:
        payload = {
            "workflow_runs": [
                run(2, "release", "workflow_dispatch"),
                run(9, "other", "push", conclusion="failure"),
                run(1, "release", "push"),
            ]
        }
        self.assertEqual(len(qualifying_runs(payload, "release", self.jobs)), 1)

    def test_rejects_dispatch_without_push(self) -> None:
        with self.assertRaises(SystemExit):
            qualifying_runs(
                {"workflow_runs": [run(1, "release", "workflow_dispatch")]},
                "release",
                self.jobs,
            )

    def test_rejects_wrong_sha_and_insufficient_history(self) -> None:
        with self.assertRaises(SystemExit):
            qualifying_runs(
                {"workflow_runs": [run(1, "other", "push")]},
                "release",
                self.jobs,
            )

    def test_rejects_failed_run(self) -> None:
        payload = {
            "workflow_runs": [run(1, "release", "push", conclusion="failure")]
        }
        with self.assertRaises(SystemExit):
            qualifying_runs(payload, "release", self.jobs)

    def test_rejects_missing_failed_or_skipped_job(self) -> None:
        payload = {"workflow_runs": [run(1, "release", "push")]}
        for jobs in [
            {"jobs": self.jobs[1]["jobs"][:-1]},
            {
                "jobs": [
                    *self.jobs[1]["jobs"][:-1],
                    {"name": "spellbook-smoke", "status": "completed", "conclusion": "skipped"},
                ]
            },
        ]:
            with self.subTest(jobs=jobs):
                with self.assertRaises(SystemExit):
                    qualifying_runs(payload, "release", {**self.jobs, 1: jobs})


def run(
    run_id: int,
    sha: str,
    event: str,
    *,
    conclusion: str = "success",
) -> dict[str, object]:
    return {
        "id": run_id,
        "head_sha": sha,
        "event": event,
        "status": "completed",
        "conclusion": conclusion,
    }


if __name__ == "__main__":
    unittest.main()
