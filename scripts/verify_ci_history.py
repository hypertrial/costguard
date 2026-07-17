#!/usr/bin/env python3
"""Require one successful exact-SHA GitHub Actions run for a release commit."""

from __future__ import annotations

import argparse
import json
import os
import re
from typing import Any
from urllib.parse import parse_qsl, quote, urlencode, urlsplit, urlunsplit
from urllib.request import Request, urlopen

REQUIRED_JOBS = ("pr-gate", "scale", "spellbook-smoke", "nba-monte-carlo-smoke")


def qualifying_runs(
    payload: dict[str, object],
    sha: str,
    jobs_by_run: dict[int, dict[str, object]],
    *,
    event: str = "push",
    required_jobs: tuple[str, ...] = REQUIRED_JOBS,
) -> list[dict[str, object]]:
    runs = payload.get("workflow_runs")
    if not isinstance(runs, list):
        raise SystemExit("GitHub Actions response did not contain workflow_runs")
    matching = [
        run
        for run in runs
        if isinstance(run, dict)
        and run.get("head_sha") == sha
        and run.get("event") == event
    ]
    selected = matching[:1]
    if not selected:
        raise SystemExit(f"commit {sha} has no completed {event} workflow run")
    failed = [
        str(run.get("html_url", run.get("id", "unknown")))
        for run in selected
        if run.get("status") != "completed" or run.get("conclusion") != "success"
    ]
    if failed:
        raise SystemExit(
            f"the latest exact-SHA {event} workflow run did not complete successfully: "
            + ", ".join(failed)
        )
    for run in selected:
        run_id = run.get("id")
        if not isinstance(run_id, int):
            raise SystemExit("GitHub Actions response contained a run without an integer id")
        jobs_payload = jobs_by_run.get(run_id)
        if jobs_payload is None:
            raise SystemExit(f"missing jobs response for CI run {run_id}")
        require_successful_jobs(run_id, jobs_payload, required_jobs)
    return selected


def require_successful_jobs(
    run_id: int,
    payload: dict[str, object],
    required_jobs: tuple[str, ...] = REQUIRED_JOBS,
) -> None:
    jobs = payload.get("jobs")
    if not isinstance(jobs, list):
        raise SystemExit(f"GitHub Actions response for run {run_id} did not contain jobs")
    by_name = {
        str(job.get("name")): job
        for job in jobs
        if isinstance(job, dict) and isinstance(job.get("name"), str)
    }
    missing = [name for name in required_jobs if name not in by_name]
    if missing:
        raise SystemExit(f"CI run {run_id} is missing required jobs: {', '.join(missing)}")
    failed = [
        name
        for name in required_jobs
        if by_name[name].get("status") != "completed"
        or by_name[name].get("conclusion") != "success"
    ]
    if failed:
        raise SystemExit(
            f"CI run {run_id} has unsuccessful or skipped required jobs: "
            + ", ".join(failed)
        )


def github_json(url: str, token: str) -> dict[str, Any]:
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
    if not isinstance(payload, dict):
        raise SystemExit(f"GitHub API returned an invalid response for {url}")
    return payload


def github_paginated(
    url: str,
    token: str,
    list_key: str,
    *,
    fetch: Any = github_json,
) -> dict[str, object]:
    combined: list[object] = []
    page = 1
    while True:
        parts = urlsplit(url)
        query = dict(parse_qsl(parts.query))
        query.update({"per_page": "100", "page": str(page)})
        page_url = urlunsplit(
            (parts.scheme, parts.netloc, parts.path, urlencode(query), parts.fragment)
        )
        payload = fetch(page_url, token)
        items = payload.get(list_key)
        if not isinstance(items, list):
            raise SystemExit(f"GitHub Actions response did not contain {list_key}")
        combined.extend(items)
        if len(items) < 100:
            return {list_key: combined}
        page += 1


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repository", required=True)
    parser.add_argument("--sha", required=True)
    parser.add_argument("--workflow", default="ci.yml")
    parser.add_argument("--event", default="push")
    parser.add_argument("--required-job", action="append", dest="required_jobs")
    args = parser.parse_args()
    if not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", args.repository):
        raise SystemExit("--repository must use owner/name format")
    token = os.environ.get("GH_TOKEN")
    if not token:
        raise SystemExit("GH_TOKEN is required")
    required_jobs = tuple(args.required_jobs or REQUIRED_JOBS)
    api_url = os.environ.get("GITHUB_API_URL", "https://api.github.com").rstrip("/")
    url = (
        f"{api_url}/repos/{args.repository}/actions/workflows/"
        f"{quote(args.workflow, safe='')}/runs?status=completed"
    )
    payload = github_paginated(url, token, "workflow_runs")
    matching_ids = [
        run.get("id")
        for run in payload.get("workflow_runs", [])
        if isinstance(run, dict)
        and run.get("head_sha") == args.sha
        and run.get("event") == args.event
    ][:1]
    jobs_by_run = {
        run_id: github_paginated(
            f"{api_url}/repos/{args.repository}/actions/runs/{run_id}/jobs",
            token,
            "jobs",
        )
        for run_id in matching_ids
        if isinstance(run_id, int)
    }
    qualifying_runs(
        payload,
        args.sha,
        jobs_by_run,
        event=args.event,
        required_jobs=required_jobs,
    )
    print(
        f"verified one exact-SHA {args.event} {args.workflow} run with required jobs "
        f"for {args.sha}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
