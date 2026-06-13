#!/usr/bin/env python3
"""Build, inspect, scan, and Compose-smoke the Costguard server image."""

from __future__ import annotations

import argparse
import json
import os
import secrets
import subprocess
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def run(command: list[str], *, env: dict[str, str] | None = None, capture: bool = False) -> str:
    print("+", " ".join(command), flush=True)
    completed = subprocess.run(
        command,
        cwd=ROOT,
        env=env,
        check=True,
        text=True,
        capture_output=capture,
    )
    return completed.stdout.strip() if capture else ""


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--version", default="2.0.0")
    parser.add_argument("--skip-scan", action="store_true")
    args = parser.parse_args()
    image = f"costguard-server:{args.version}"
    run(["docker", "build", "--file", "Dockerfile.server", "--tag", image, "."])
    user = run(["docker", "image", "inspect", image, "--format", "{{.Config.User}}"], capture=True)
    if user != "10001:10001":
        raise SystemExit(f"server image must run as 10001:10001, got {user!r}")
    if not args.skip_scan:
        run(
            [
                "docker",
                "run",
                "--rm",
                "-v",
                "/var/run/docker.sock:/var/run/docker.sock",
                "aquasec/trivy:0.62.1",
                "image",
                "--exit-code",
                "1",
                "--severity",
                "CRITICAL,HIGH",
                "--ignore-unfixed",
                image,
            ]
        )

    project = f"costguard-smoke-{secrets.token_hex(4)}"
    environment = os.environ.copy()
    environment.update(
        {
            "COMPOSE_PROJECT_NAME": project,
            "POSTGRES_PASSWORD": secrets.token_urlsafe(32),
            "COSTGUARD_BOOTSTRAP_SECRET": secrets.token_urlsafe(32),
            "COSTGUARD_PUBLIC_URL": "http://127.0.0.1:18080",
            "COSTGUARD_PORT": "18080",
        }
    )
    try:
        run(["docker", "compose", "up", "--detach", "--build", "--wait"], env=environment)
        with urllib.request.urlopen("http://127.0.0.1:18080/readyz", timeout=10) as response:
            if response.status != 200:
                raise SystemExit(f"unexpected readiness status {response.status}")
        request = urllib.request.Request(
            "http://127.0.0.1:18080/api/v1/bootstrap",
            method="POST",
            data=json.dumps(
                {
                    "organization_slug": "smoke",
                    "organization_name": "Smoke",
                    "owner_email": "owner@example.com",
                    "owner_name": "Owner",
                }
            ).encode(),
            headers={
                "Content-Type": "application/json",
                "X-Costguard-Bootstrap-Secret": environment["COSTGUARD_BOOTSTRAP_SECRET"],
            },
        )
        with urllib.request.urlopen(request, timeout=15) as response:
            payload = json.load(response)
        if not str(payload.get("token", "")).startswith("cg_"):
            raise SystemExit("bootstrap did not return a service token")
        print("server Compose smoke passed")
    finally:
        subprocess.run(
            ["docker", "compose", "down", "--volumes", "--remove-orphans"],
            cwd=ROOT,
            env=environment,
            check=False,
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
