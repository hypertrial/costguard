#!/usr/bin/env python3
"""Ensure workspace.dependencies matches the approved direct dependency set."""

from __future__ import annotations

import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CARGO_TOML = ROOT / "Cargo.toml"

ALLOWED = frozenset(
    {
        "anyhow",
        "base64",
        "chrono",
        "clap",
        "ed25519-dalek",
        "globset",
        "ignore",
        "getrandom",
        "rayon",
        "regex",
        "schemars",
        "serde",
        "serde_json",
        "serde_yaml",
        "sha2",
        "sqlparser",
        "tempfile",
        "toml",
    }
)

def workspace_dependency_keys() -> set[str]:
    data = tomllib.loads(CARGO_TOML.read_text(encoding="utf-8"))
    deps = data.get("workspace", {}).get("dependencies", {})
    if not isinstance(deps, dict):
        raise ValueError("workspace.dependencies must be a table")
    return set(deps.keys())


def main() -> int:
    if not CARGO_TOML.exists():
        print(f"missing Cargo.toml: {CARGO_TOML}", file=sys.stderr)
        return 1

    actual = workspace_dependency_keys()
    unexpected = sorted(actual - ALLOWED)
    missing = sorted(ALLOWED - actual)

    if unexpected or missing:
        if unexpected:
            print("unexpected workspace.dependencies entries:", file=sys.stderr)
            for name in unexpected:
                print(f"  + {name}", file=sys.stderr)
        if missing:
            print("missing expected workspace.dependencies entries:", file=sys.stderr)
            for name in missing:
                print(f"  - {name}", file=sys.stderr)
        print(
            "\nUpdate scripts/validate_workspace_deps.py ALLOWED when intentionally "
            "adding or removing a direct dependency.",
            file=sys.stderr,
        )
        return 1

    print(f"workspace.dependencies ok ({len(actual)} allowed entries)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
