#!/usr/bin/env python3
"""Generate a deterministic CycloneDX SBOM from Cargo metadata."""

from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
import uuid
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[1]


def cargo_metadata() -> dict[str, Any]:
    completed = subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--locked"],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=True,
    )
    return json.loads(completed.stdout)


def purl(name: str, version: str) -> str:
    return f"pkg:cargo/{name}@{version}"


def build_sbom(metadata: dict[str, Any], lock_bytes: bytes) -> dict[str, Any]:
    packages = {package["id"]: package for package in metadata["packages"]}
    components = []
    for package in sorted(packages.values(), key=lambda item: (item["name"], item["version"])):
        component: dict[str, Any] = {
            "type": "library",
            "bom-ref": purl(package["name"], package["version"]),
            "name": package["name"],
            "version": package["version"],
            "purl": purl(package["name"], package["version"]),
        }
        if package.get("license"):
            component["licenses"] = [{"expression": package["license"]}]
        components.append(component)

    dependencies = []
    for node in sorted(metadata.get("resolve", {}).get("nodes", []), key=lambda item: item["id"]):
        package = packages[node["id"]]
        depends_on = sorted(
            purl(packages[dependency["pkg"]]["name"], packages[dependency["pkg"]]["version"])
            for dependency in node.get("deps", [])
        )
        dependencies.append(
            {
                "ref": purl(package["name"], package["version"]),
                "dependsOn": depends_on,
            }
        )

    workspace = next(
        package
        for package in packages.values()
        if package["name"] == "costguard-cli"
    )
    digest = hashlib.sha256(lock_bytes).hexdigest()
    serial = uuid.uuid5(uuid.NAMESPACE_URL, f"costguard-cargo-lock:{digest}")
    return {
        "bomFormat": "CycloneDX",
        "specVersion": "1.5",
        "serialNumber": f"urn:uuid:{serial}",
        "version": 1,
        "metadata": {
            "component": {
                "type": "application",
                "bom-ref": purl("costguard", workspace["version"]),
                "name": "costguard",
                "version": workspace["version"],
                "purl": purl("costguard", workspace["version"]),
            },
            "properties": [
                {"name": "costguard:cargo-lock-sha256", "value": digest},
            ],
        },
        "components": components,
        "dependencies": dependencies,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=Path("dist/release/costguard.cdx.json"))
    args = parser.parse_args()
    payload = build_sbom(cargo_metadata(), (ROOT / "Cargo.lock").read_bytes())
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(
        json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    print(f"wrote {args.output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
