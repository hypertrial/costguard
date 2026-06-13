#!/usr/bin/env python3
"""Require consecutive successful CI runs for a release commit."""

from __future__ import annotations

import argparse
import json
import os
from urllib.request import Request, urlopen


def successful_runs(payload: dict[str, object], sha: str, required: int) -> list[dict[str, object]]:
    runs = payload.get("workflow_runs")
    if not isinstance(runs, list):
        raise SystemExit("GitHub Actions response did not contain workflow_runs")
    matching = [run for run in runs if isinstance(run, dict) and run.get("head_sha") == sha]
    selected = matching[:required]
    if len(selected) < required:
        raise SystemExit(f"commit {sha} has {len(selected)} completed CI runs; {required} required")
    failed = [str(run.get("html_url", run.get("id", "unknown"))) for run in selected if run.get("conclusion") != "success"]
    if failed:
        raise SystemExit("the most recent CI runs for the release commit are not all green: " + ", ".join(failed))
    return selected


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repository", required=True)
    parser.add_argument("--sha", required=True)
    parser.add_argument("--workflow", default="ci.yml")
    parser.add_argument("--required", type=int, default=3)
    args = parser.parse_args()
    if args.required < 1:
        raise SystemExit("--required must be positive")
    token = os.environ.get("GH_TOKEN")
    if not token:
        raise SystemExit("GH_TOKEN is required")
    url = (
        f"https://api.github.com/repos/{args.repository}/actions/workflows/"
        f"{args.workflow}/runs?status=completed&per_page=100"
    )
    request = Request(
        url,
        headers={
            "Accept": "application/vnd.github+json",
            "Authorization": f"Bearer {token}",
            "X-GitHub-Api-Version": "2022-11-28",
        },
    )
    with urlopen(request, timeout=30) as response:  # noqa: S310 - fixed GitHub API origin
        payload = json.load(response)
    selected = successful_runs(payload, args.sha, args.required)
    print(f"verified {len(selected)} consecutive successful {args.workflow} runs for {args.sha}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
