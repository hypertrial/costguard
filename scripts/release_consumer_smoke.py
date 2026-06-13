#!/usr/bin/env python3
"""Exercise the exact release archive through the composite Action driver."""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
DRIVER = ROOT / ".github/actions/costguard/scripts/costguard_action.py"
sys.path.insert(0, str(ROOT / "scripts"))

from costguard_tooling import host_target  # noqa: E402


def run(command: list[str], *, cwd: Path, env: dict[str, str]) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        command,
        cwd=cwd,
        env={**os.environ, **env},
        capture_output=True,
        text=True,
        check=False,
    )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--workdir", type=Path, default=Path("dist/release"))
    parser.add_argument("--version", required=True)
    args = parser.parse_args()
    target, _bin_name = host_target()
    archive = args.workdir / f"costguard-{target}.tar.gz"
    checksum = args.workdir / f"{archive.name}.sha256"
    if not archive.is_file() or not checksum.is_file():
        raise SystemExit(f"missing host release assets for {target}")

    with tempfile.TemporaryDirectory(prefix="costguard-consumer-") as tmp:
        temp = Path(tmp)
        project = temp / "consumer"
        project.mkdir()
        subprocess.run(["git", "init"], cwd=project, check=True, capture_output=True)
        subprocess.run(
            ["git", "config", "user.email", "costguard@example.com"],
            cwd=project,
            check=True,
        )
        subprocess.run(
            ["git", "config", "user.name", "Costguard Consumer Smoke"],
            cwd=project,
            check=True,
        )
        (project / "query.sql").write_text("select 1 as id\n", encoding="utf-8")
        subprocess.run(["git", "add", "."], cwd=project, check=True)
        subprocess.run(["git", "commit", "-m", "initial"], cwd=project, check=True)
        subprocess.run(["git", "branch", "-M", "main"], cwd=project, check=True)
        subprocess.run(["git", "checkout", "-b", "feature"], cwd=project, check=True)
        (project / "query.sql").write_text("select 2 as id\n", encoding="utf-8")

        github_path = temp / "github-path"
        install = run(
            [
                sys.executable,
                str(DRIVER),
                "install",
                "--mode",
                "release",
                "--version",
                args.version,
            ],
            cwd=project,
            env={
                "GITHUB_ACTION_PATH": str(DRIVER.parents[1]),
                "COSTGUARD_RELEASE_BASE_URL": args.workdir.resolve().as_uri(),
                "VERIFY_ATTESTATION_INPUT": "false",
                "RUNNER_TEMP": str(temp / "runner"),
                "GITHUB_PATH": str(github_path),
            },
        )
        if install.returncode != 0:
            raise SystemExit(install.stderr)
        installed_path = github_path.read_text(encoding="utf-8").strip()
        scan = run(
            [sys.executable, str(DRIVER), "run"],
            cwd=project,
            env={
                "GITHUB_ACTION_PATH": str(DRIVER.parents[1]),
                "GITHUB_WORKSPACE": str(project),
                "BASE_INPUT": "main",
                "WAREHOUSE_INPUT": "generic",
                "FAIL_ON_INPUT": "critical",
                "FORMAT_INPUT": "json",
                "ANALYSIS_POLICY_INPUT": "strict",
                "PATH": f"{installed_path}{os.pathsep}{os.environ.get('PATH', '')}",
            },
        )
        if scan.returncode != 0:
            raise SystemExit(scan.stderr or scan.stdout)
        payload = json.loads(scan.stdout)
        if payload.get("schema_version") != 3 or not payload.get("analysis", {}).get("passed"):
            raise SystemExit("consumer smoke returned an invalid or incomplete scan")
    print(f"consumer repository smoke passed for {args.version}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
