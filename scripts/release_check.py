#!/usr/bin/env python3
"""Run the authoritative local Costguard release qualification gate."""

from __future__ import annotations

import argparse
import json
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))

from costguard_tooling import git_output, workspace_version  # noqa: E402

COMMANDS: list[list[str]] = []


def run(command: list[str], *, env: dict[str, str] | None = None) -> None:
    print("+", " ".join(command), flush=True)
    COMMANDS.append(command)
    subprocess.run(command, cwd=ROOT, env=env, check=True)


def require_release_tag(version: str) -> tuple[str, str]:
    if git_output("status", "--porcelain"):
        raise SystemExit("release qualification requires a clean worktree")
    tag = f"v{version}"
    if git_output("cat-file", "-t", f"refs/tags/{tag}") != "tag":
        raise SystemExit(f"release tag {tag} must be annotated")
    verify = subprocess.run(
        ["git", "tag", "-v", tag], cwd=ROOT, capture_output=True, text=True, check=False
    )
    if verify.returncode != 0:
        raise SystemExit(f"release tag signature verification failed:\n{verify.stderr}")
    commit = git_output("rev-list", "-n", "1", tag)
    head = git_output("rev-parse", "HEAD")
    if commit != head:
        raise SystemExit(f"release tag {tag} points to {commit}, not HEAD {head}")
    return tag, commit


def require_release_tools() -> None:
    missing = [name for name in ["cargo-deny", "mdbook"] if shutil.which(name) is None]
    if missing:
        raise SystemExit(f"release qualification requires: {', '.join(missing)}")


def write_receipt(path: Path, *, version: str, tag: str, commit: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(
        json.dumps(
            {
                "schema_version": 1,
                "status": "passed",
                "version": version,
                "tag": tag,
                "commit": commit,
                "commands": COMMANDS,
            },
            indent=2,
            sort_keys=True,
        )
        + "\n",
        encoding="utf-8",
    )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--version", required=True)
    parser.add_argument("--skip-external", action="store_true")
    parser.add_argument("--skip-external-links", action="store_true")
    parser.add_argument("--development", action="store_true")
    parser.add_argument("--receipt", type=Path, default=Path("dist/release/release-check.json"))
    args = parser.parse_args()
    version = args.version.removeprefix("v")
    if version != workspace_version():
        raise SystemExit(f"requested version {version} != workspace version {workspace_version()}")
    tag = f"v{version}"
    commit = git_output("rev-parse", "HEAD")
    if args.development:
        print("development qualification does not produce release evidence")
    else:
        if args.skip_external or args.skip_external_links:
            raise SystemExit("release evidence cannot be created with skip flags")
        tag, commit = require_release_tag(version)
        require_release_tools()

    run(["./scripts/ci_local.sh", "--spellbook-smoke"] if not args.skip_external else ["./scripts/ci_local.sh"])
    run([sys.executable, "scripts/scale_check.py"])
    if not args.skip_external:
        run([sys.executable, "scripts/benchmark_external_repo.py", "--repo", "jaffle-shop", "--force-compile"])
        run([sys.executable, "scripts/benchmark_external_repo.py", "--repo", "spellbook", "--force-compile"])
    if not args.skip_external_links:
        run([sys.executable, "scripts/check_docs.py", "--external"])
    if not args.development:
        write_receipt(args.receipt, version=version, tag=tag, commit=commit)
        print(f"wrote qualification receipt {args.receipt}")
    print(f"release qualification passed for {version}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
