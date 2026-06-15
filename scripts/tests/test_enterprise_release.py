#!/usr/bin/env python3

from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]

import sys

sys.path.insert(0, str(ROOT / "scripts"))

from costguard_tooling import file_sha256  # noqa: E402
from generate_sbom import build_sbom  # noqa: E402
from release_packaging import RELEASE_TARGETS, asset_name  # noqa: E402
from verify_release_inventory import verify  # noqa: E402


class EnterpriseReleaseTest(unittest.TestCase):
    def test_sbom_is_deterministic_cyclonedx(self) -> None:
        metadata = {
            "packages": [
                {
                    "id": "costguard-cli 2.2.0",
                    "name": "costguard-cli",
                    "version": "2.2.0",
                    "license": "MIT",
                },
                {
                    "id": "serde 1.0.0",
                    "name": "serde",
                    "version": "1.0.0",
                    "license": "MIT OR Apache-2.0",
                },
            ],
            "resolve": {
                "nodes": [
                    {
                        "id": "costguard-cli 2.2.0",
                        "deps": [{"pkg": "serde 1.0.0"}],
                    },
                    {"id": "serde 1.0.0", "deps": []},
                ]
            },
        }
        first = build_sbom(metadata, b"lock")
        second = build_sbom(metadata, b"lock")
        self.assertEqual(first, second)
        self.assertEqual(first["bomFormat"], "CycloneDX")
        self.assertEqual(first["metadata"]["component"]["version"], "2.2.0")

    def test_release_inventory_requires_every_target_and_receipt(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            workdir = Path(tmp)
            for target in RELEASE_TARGETS:
                archive = workdir / asset_name(target)
                archive.write_bytes(target.encode("utf-8"))
                digest = file_sha256(archive)
                (workdir / f"{archive.name}.sha256").write_text(
                    f"{digest}  {archive.name}\n", encoding="utf-8"
                )
                (workdir / f"smoke-{target}.json").write_text(
                    json.dumps(
                        {"target": target, "version": "2.2.0", "sha256": digest}
                    ),
                    encoding="utf-8",
                )
            (workdir / "costguard.cdx.json").write_text("{}\n", encoding="utf-8")
            (workdir / "release-check.json").write_text("{}\n", encoding="utf-8")
            sums = verify(workdir, "2.2.0")
            self.assertTrue(sums.exists())


if __name__ == "__main__":
    unittest.main()
