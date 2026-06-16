#!/usr/bin/env python3

from __future__ import annotations

import hashlib
import os
import platform
import subprocess
import tarfile
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
INSTALL_SH = ROOT / "scripts" / "install.sh"


def host_target() -> str:
    system = platform.system()
    machine = platform.machine().lower()
    if system == "Linux" and machine in {"x86_64", "amd64"}:
        return "x86_64-unknown-linux-gnu"
    if system == "Darwin" and machine == "arm64":
        return "aarch64-apple-darwin"
    if system == "Darwin" and machine == "x86_64":
        return "x86_64-apple-darwin"
    raise unittest.SkipTest(f"unsupported host platform: {system}-{machine}")


class InstallShTest(unittest.TestCase):
    def test_syntax(self) -> None:
        completed = subprocess.run(
            ["sh", "-n", str(INSTALL_SH)],
            capture_output=True,
            text=True,
            check=False,
        )
        self.assertEqual(completed.returncode, 0, completed.stderr)

    def test_installs_from_local_release_fixture(self) -> None:
        target = host_target()
        asset = f"costguard-{target}.tar.gz"
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            release_dir = tmp_path / "release"
            install_dir = tmp_path / "bin"
            release_dir.mkdir()
            install_dir.mkdir()

            fake_bin = tmp_path / "costguard"
            fake_bin.write_text("#!/bin/sh\necho costguard test 1.2.3\n", encoding="utf-8")
            fake_bin.chmod(0o755)

            archive = release_dir / asset
            with tarfile.open(archive, "w:gz") as tar:
                tar.add(fake_bin, arcname="costguard")

            digest = hashlib.sha256(archive.read_bytes()).hexdigest()
            (release_dir / f"{asset}.sha256").write_text(
                f"{digest}  {asset}\n",
                encoding="utf-8",
            )

            env = os.environ.copy()
            env["COSTGUARD_RELEASE_BASE_URL"] = release_dir.as_uri()
            env["COSTGUARD_INSTALL_DIR"] = str(install_dir)
            env["COSTGUARD_VERSION"] = "v0.0.0-test"

            completed = subprocess.run(
                ["sh", str(INSTALL_SH)],
                capture_output=True,
                text=True,
                check=False,
                env=env,
            )
            self.assertEqual(completed.returncode, 0, completed.stderr + completed.stdout)
            installed = install_dir / "costguard"
            self.assertTrue(installed.is_file())
            self.assertIn("costguard test 1.2.3", completed.stdout)

    def test_rejects_checksum_mismatch(self) -> None:
        target = host_target()
        asset = f"costguard-{target}.tar.gz"
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            release_dir = tmp_path / "release"
            install_dir = tmp_path / "bin"
            release_dir.mkdir()
            install_dir.mkdir()

            fake_bin = tmp_path / "costguard"
            fake_bin.write_text("#!/bin/sh\necho ok\n", encoding="utf-8")
            fake_bin.chmod(0o755)

            archive = release_dir / asset
            with tarfile.open(archive, "w:gz") as tar:
                tar.add(fake_bin, arcname="costguard")

            (release_dir / f"{asset}.sha256").write_text(
                f"{'0' * 64}  {asset}\n",
                encoding="utf-8",
            )

            env = os.environ.copy()
            env["COSTGUARD_RELEASE_BASE_URL"] = release_dir.as_uri()
            env["COSTGUARD_INSTALL_DIR"] = str(install_dir)

            completed = subprocess.run(
                ["sh", str(INSTALL_SH)],
                capture_output=True,
                text=True,
                check=False,
                env=env,
            )
            self.assertNotEqual(completed.returncode, 0)
            self.assertIn("checksum mismatch", completed.stderr)


if __name__ == "__main__":
    unittest.main()
