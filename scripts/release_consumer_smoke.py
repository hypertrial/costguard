#!/usr/bin/env python3
"""Exercise the exact release archive through the composite Action driver."""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import tempfile
from datetime import UTC, datetime, timedelta
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
    parser.add_argument(
        "--published",
        action="store_true",
        help="download the published GitHub release and verify its attestation",
    )
    args = parser.parse_args()
    target, bin_name = host_target()
    if not args.published:
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
        install_env = {
            "GITHUB_ACTION_PATH": str(DRIVER.parents[1]),
            "RUNNER_TEMP": str(temp / "runner"),
            "GITHUB_PATH": str(github_path),
        }
        if args.published:
            install_env["VERIFY_ATTESTATION_INPUT"] = "true"
        else:
            install_env.update(
                {
                    "COSTGUARD_RELEASE_BASE_URL": args.workdir.resolve().as_uri(),
                    "VERIFY_ATTESTATION_INPUT": "false",
                }
            )
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
            env=install_env,
        )
        if install.returncode != 0:
            raise SystemExit(install.stderr)
        installed_path = github_path.read_text(encoding="utf-8").strip()
        common_env = {
            "GITHUB_ACTION_PATH": str(DRIVER.parents[1]),
            "GITHUB_WORKSPACE": str(project),
            "BASE_INPUT": "main",
            "WAREHOUSE_INPUT": "generic",
            "FAIL_ON_INPUT": "critical",
            "FORMAT_INPUT": "json",
            "PATH": f"{installed_path}{os.pathsep}{os.environ.get('PATH', '')}",
        }
        scan = run(
            [sys.executable, str(DRIVER), "run"],
            cwd=project,
            env={
                **common_env,
                "ANALYSIS_POLICY_INPUT": "standard",
            },
        )
        if scan.returncode != 0:
            raise SystemExit(scan.stderr or scan.stdout)
        payload = json.loads(scan.stdout)
        if payload.get("schema_version") != 4 or not payload.get("analysis", {}).get("passed"):
            raise SystemExit("consumer smoke returned an invalid or incomplete scan")
        policy_dir = project / ".costguard"
        policy_dir.mkdir()
        now = datetime.now(UTC)
        (policy_dir / "policy.toml").write_text(
            "\n".join(
                [
                    "schema_version = 2",
                    'identity_scheme = "semantic-v1"',
                    'id = "consumer-enterprise"',
                    'version = "rc-smoke"',
                    'organization = "smoke"',
                    f'issued_at = "{(now - timedelta(minutes=1)).isoformat()}"',
                    f'expires_at = "{(now + timedelta(days=1)).isoformat()}"',
                    "",
                    "[[scopes]]",
                    'id = "repository"',
                    'kind = "repository"',
                    'selector = "smoke/consumer"',
                    "priority = 0",
                    'enforcement = "block"',
                    "",
                ]
            ),
            encoding="utf-8",
        )
        binary = Path(installed_path) / bin_name
        policy_commands = [
            [
                str(binary),
                "policy",
                "keygen",
                "smoke-root",
                "--private-key",
                str(policy_dir / "private.json"),
                "--trust-store",
                str(policy_dir / "trust.json"),
            ],
            [
                str(binary),
                "policy",
                "compile",
                str(policy_dir / "policy.toml"),
                str(policy_dir / "policy.json"),
            ],
            [
                str(binary),
                "policy",
                "sign",
                str(policy_dir / "policy.json"),
                str(policy_dir / "private.json"),
                str(policy_dir / "policy.signed.json"),
            ],
        ]
        for command in policy_commands:
            completed = run(command, cwd=project, env={})
            if completed.returncode != 0:
                raise SystemExit(completed.stderr or completed.stdout)
        (project / "target").mkdir(exist_ok=True)
        (project / "target/manifest.json").write_text('{"nodes": {}}\n', encoding="utf-8")
        enterprise = run(
            [sys.executable, str(DRIVER), "run"],
            cwd=project,
            env={
                **common_env,
                "ANALYSIS_POLICY_INPUT": "strict",
                "POLICY_BUNDLE_INPUT": ".costguard/policy.signed.json",
                "TRUST_STORE_INPUT": ".costguard/trust.json",
                "POLICY_ORGANIZATION_INPUT": "smoke",
                "POLICY_REPOSITORY_INPUT": "smoke/consumer",
            },
        )
        if enterprise.returncode != 0:
            raise SystemExit(enterprise.stderr or enterprise.stdout)
        enterprise_payload = json.loads(enterprise.stdout)
        if (
            not enterprise_payload.get("analysis", {}).get("passed")
            or enterprise_payload.get("analysis", {}).get("policy") != "strict"
            or enterprise_payload.get("policy", {}).get("digest") == "local-unmanaged"
        ):
            raise SystemExit("enterprise consumer smoke did not enforce signed strict policy")
    print(f"consumer repository smoke passed for {args.version}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
