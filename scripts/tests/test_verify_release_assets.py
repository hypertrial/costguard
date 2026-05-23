#!/usr/bin/env python3

from __future__ import annotations

import shutil
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]

import sys

sys.path.insert(0, str(ROOT / "scripts"))

from verify_release_assets import (  # noqa: E402
    asset_name,
    extract_and_smoke_test,
    host_target,
    package_release_binary,
    verify_checksum,
    verify_release_assets,
)


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
            target, bin_name = host_target()
            asset, checksum = package_release_binary(workdir, target=target, bin_name=bin_name)
            self.assertTrue(asset.exists())
            self.assertTrue(checksum.exists())
            verify_checksum(workdir, asset, checksum)
            extract_and_smoke_test(asset, bin_name=bin_name)

    def test_verify_release_assets_entrypoint(self) -> None:
        if shutil.which("shasum") is None:
            self.skipTest("shasum not available")
        verify_release_assets()


if __name__ == "__main__":
    unittest.main()
