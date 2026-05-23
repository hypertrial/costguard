#!/usr/bin/env python3

from __future__ import annotations

import shutil
import tempfile
import unittest
from pathlib import Path
from unittest import mock

ROOT = Path(__file__).resolve().parents[2]

import sys

sys.path.insert(0, str(ROOT / "scripts"))

import publish_release_local  # noqa: E402
from release_packaging import (  # noqa: E402
    RELEASE_TARGETS,
    asset_name,
    package_built_binary,
    target_bin_name,
    verify_checksum,
)


class PublishReleaseLocalTest(unittest.TestCase):
    def test_release_targets_and_asset_names(self) -> None:
        self.assertEqual(len(RELEASE_TARGETS), 4)
        for target in RELEASE_TARGETS:
            self.assertEqual(asset_name(target), f"costguard-{target}.tar.gz")
            self.assertIn(
                target_bin_name(target),
                {"costguard", "costguard.exe"},
            )

    def test_package_only_builds_all_targets(self) -> None:
        if shutil.which("shasum") is None:
            self.skipTest("shasum not available")
        with tempfile.TemporaryDirectory() as tmp:
            workdir = Path(tmp)
            root = workdir / "root"
            root.mkdir()
            stub = root / "target" / RELEASE_TARGETS[0] / "release" / "costguard"
            stub.parent.mkdir(parents=True)
            stub.write_text(
                "#!/usr/bin/env python3\n"
                "import json, sys\n"
                "if sys.argv[1:3] == ['rules', '--format']:\n"
                "    json.dump([{'id': 'SQLCOST001'}], sys.stdout)\n"
                "    raise SystemExit(0)\n"
                "raise SystemExit(2)\n",
                encoding="utf-8",
            )
            stub.chmod(0o755)

            def fake_build(root_path: Path, target: str) -> Path:
                path = root_path / "target" / target / "release" / target_bin_name(target)
                path.parent.mkdir(parents=True, exist_ok=True)
                if path != stub:
                    shutil.copy2(stub, path)
                path.chmod(0o755)
                return path

            with mock.patch(
                "publish_release_local.ensure_targets_installed"
            ), mock.patch(
                "publish_release_local.package_and_verify_target",
                side_effect=lambda root_path, out, target: (
                    package_built_binary(
                        out,
                        target=target,
                        binary_path=fake_build(root_path, target),
                    )
                ),
            ):
                assets = publish_release_local.package_all_targets(root, workdir)

            self.assertEqual(len(assets), len(RELEASE_TARGETS) * 2)
            for target in RELEASE_TARGETS:
                self.assertTrue((workdir / asset_name(target)).exists())

    def test_ensure_targets_installed_fails_when_missing(self) -> None:
        with mock.patch(
            "release_packaging.installed_rust_targets",
            return_value={"aarch64-apple-darwin"},
        ):
            with self.assertRaises(SystemExit):
                from release_packaging import ensure_targets_installed

                ensure_targets_installed(RELEASE_TARGETS)

    def test_publish_requires_all_assets(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            workdir = Path(tmp)
            with self.assertRaises(SystemExit):
                publish_release_local.publish_release("v0.1.0", workdir, notes_file=None)


if __name__ == "__main__":
    unittest.main()
