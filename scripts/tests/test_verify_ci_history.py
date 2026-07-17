#!/usr/bin/env python3

from __future__ import annotations

import sys
import unittest
from pathlib import Path
from urllib.parse import parse_qs, urlsplit

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))

from verify_ci_history import github_paginated, qualifying_runs  # noqa: E402


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

    def test_accepts_dispatch_with_custom_required_job(self) -> None:
        payload = {"workflow_runs": [run(2, "release", "workflow_dispatch")]}
        jobs = {
            2: {
                "jobs": [
                    {
                        "name": "full-evidence-gate",
                        "status": "completed",
                        "conclusion": "success",
                    }
                ]
            }
        }

        selected = qualifying_runs(
            payload,
            "release",
            jobs,
            event="workflow_dispatch",
            required_jobs=("full-evidence-gate",),
        )

        self.assertEqual([item["id"] for item in selected], [2])

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

    def test_latest_exact_sha_failure_is_not_hidden_by_older_success(self) -> None:
        payload = {
            "workflow_runs": [
                run(2, "release", "push", conclusion="failure"),
                run(1, "release", "push"),
            ]
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

    def test_paginates_workflow_runs_and_jobs(self) -> None:
        calls: list[str] = []

        def fetch(url: str, _token: str) -> dict[str, object]:
            calls.append(url)
            if parse_qs(urlsplit(url).query).get("page") == ["1"]:
                return {"workflow_runs": [{"id": index} for index in range(100)]}
            return {"workflow_runs": [{"id": 100}]}

        payload = github_paginated(
            "https://example.test/actions/runs?status=completed",
            "secret",
            "workflow_runs",
            fetch=fetch,
        )

        self.assertEqual(len(payload["workflow_runs"]), 101)
        self.assertEqual(len(calls), 2)
        self.assertTrue(all("per_page=100" in url for url in calls))
        self.assertNotIn("secret", "".join(calls))


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
