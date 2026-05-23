#!/usr/bin/env python3

from __future__ import annotations

import shutil
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]

import sys

sys.path.insert(0, str(ROOT / "scripts"))

from costguard_tooling import costguard_binary  # noqa: E402
from release_packaging import (  # noqa: E402
    asset_name,
    extract_and_smoke_test,
    host_target,
    package_built_binary,
    target_bin_name,
    verify_checksum,
)
from verify_release_assets import verify_release_assets  # noqa: E402


class VerifyReleaseAssetsTest(unittest.TestCase):
    def test_host_target_and_asset_name(self) -> None:
        target, bin_name = host_target()
        self.assertIn(bin_name, {"costguard", "costguard.exe"})
        self.assertEqual(asset_name(target), f"costguard-{target}.tar.gz")

    def test_package_verify_and_extract_real_binary(self) -> None:
        if shutil.which("shasum") is None:
            self.skipTest("shasum not available")
        with tempfile.TemporaryDirectory() as tmp:
            workdir = Path(tmp)
            target, _ = host_target()
            asset, checksum = package_built_binary(
                workdir,
                target=target,
                binary_path=costguard_binary(release=True),
            )
            self.assertTrue(asset.exists())
            self.assertTrue(checksum.exists())
            verify_checksum(workdir, asset, checksum)
            extract_and_smoke_test(asset, bin_name=target_bin_name(target), target=target)

    def test_verify_release_assets_entrypoint(self) -> None:
        if shutil.which("shasum") is None:
            self.skipTest("shasum not available")
        verify_release_assets()


if __name__ == "__main__":
    unittest.main()
