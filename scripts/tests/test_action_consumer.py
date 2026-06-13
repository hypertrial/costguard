#!/usr/bin/env python3

from __future__ import annotations

import contextlib
import functools
import hashlib
import http.server
import importlib.util
import json
import os
import subprocess
import sys
import tarfile
import tempfile
import threading
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
ACTION_PATH = ROOT / ".github" / "actions" / "costguard"
DRIVER = ACTION_PATH / "scripts" / "costguard_action.py"


def load_driver_module():
    spec = importlib.util.spec_from_file_location("costguard_action", DRIVER)
    if spec is None or spec.loader is None:
        raise AssertionError("failed to load Action driver")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def run_driver(command: list[str], *, env: dict[str, str]) -> subprocess.CompletedProcess[str]:
    merged = os.environ.copy()
    merged.update(env)
    merged["GITHUB_ACTION_PATH"] = str(ACTION_PATH)
    return subprocess.run(
        [sys.executable, str(DRIVER), *command],
        env=merged,
        capture_output=True,
        text=True,
        check=False,
    )


@contextlib.contextmanager
def file_server(root: Path):
    handler = functools.partial(http.server.SimpleHTTPRequestHandler, directory=str(root))
    try:
        server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), handler)
    except PermissionError:
        yield root.as_uri()
        return
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        yield f"http://127.0.0.1:{server.server_port}"
    finally:
        server.shutdown()
        thread.join()
        server.server_close()


def git(root: Path, *args: str) -> None:
    subprocess.run(["git", *args], cwd=root, check=True, capture_output=True, text=True)


class ActionConsumerTest(unittest.TestCase):
    def test_floating_major_action_uses_exact_workspace_release(self) -> None:
        self.assertEqual(load_driver_module().action_release_version(), "v2.0.0")

    def test_release_install_from_local_server(self) -> None:
        binary = ROOT / "target" / "release" / "costguard"
        if not binary.exists():
            subprocess.run(
                ["cargo", "build", "--release", "--locked", "-p", "costguard-cli"],
                cwd=ROOT,
                check=True,
            )
        target = platform_target()
        asset_name = f"costguard-{target}.tar.gz"
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            asset = root / asset_name
            with tarfile.open(asset, "w:gz") as archive:
                archive.add(binary, arcname="costguard")
            digest = hashlib.sha256(asset.read_bytes()).hexdigest()
            (root / f"{asset_name}.sha256").write_text(
                f"{digest}  {asset_name}\n", encoding="utf-8"
            )
            github_path = root / "github-path"
            with file_server(root) as base_url:
                completed = run_driver(
                    ["install", "--mode", "release", "--version", "v2.0.0"],
                    env={
                        "COSTGUARD_RELEASE_BASE_URL": base_url,
                        "RUNNER_TEMP": str(root / "runner"),
                        "GITHUB_PATH": str(github_path),
                        "VERIFY_ATTESTATION_INPUT": "false",
                    },
                )
            self.assertEqual(completed.returncode, 0, completed.stderr)
            installed = Path(github_path.read_text(encoding="utf-8").strip()) / "costguard"
            output = subprocess.run(
                [str(installed), "rules", "--format", "json"],
                capture_output=True,
                text=True,
                check=True,
            )
            self.assertTrue(json.loads(output.stdout))

    def test_release_install_rejects_bad_checksum(self) -> None:
        target = platform_target()
        asset_name = f"costguard-{target}.tar.gz"
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / asset_name).write_bytes(b"not an archive")
            (root / f"{asset_name}.sha256").write_text(
                f"{'0' * 64}  {asset_name}\n", encoding="utf-8"
            )
            with file_server(root) as base_url:
                completed = run_driver(
                    ["install", "--mode", "release"],
                    env={
                        "COSTGUARD_RELEASE_BASE_URL": base_url,
                        "RUNNER_TEMP": str(root / "runner"),
                        "GITHUB_PATH": str(root / "github-path"),
                        "VERIFY_ATTESTATION_INPUT": "false",
                    },
                )
            self.assertNotEqual(completed.returncode, 0)
            self.assertIn("checksum mismatch", completed.stderr)

    def test_source_install_uses_action_repository(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            github_path = root / "github-path"
            bin_dir = root / "bin"
            bin_dir.mkdir()
            fake_cargo = bin_dir / "cargo"
            fake_cargo.write_text(
                "#!/bin/sh\n"
                f"test \"$PWD\" = \"{ROOT}\" || exit 9\n"
                "exit 0\n",
                encoding="utf-8",
            )
            fake_cargo.chmod(0o755)
            completed = run_driver(
                ["install", "--mode", "source"],
                env={
                    "GITHUB_PATH": str(github_path),
                    "GITHUB_WORKSPACE": tmp,
                    "PATH": f"{bin_dir}{os.pathsep}{os.environ['PATH']}",
                },
            )
            self.assertEqual(completed.returncode, 0, completed.stderr)
            self.assertEqual(
                Path(github_path.read_text(encoding="utf-8").strip()),
                ROOT / "target" / "release",
            )

    def test_run_auto_detects_manifest_in_working_directory(self) -> None:
        binary_dir = ROOT / "target" / "release"
        if not (binary_dir / "costguard").exists():
            subprocess.run(
                ["cargo", "build", "--release", "--locked", "-p", "costguard-cli"],
                cwd=ROOT,
                check=True,
            )
        with tempfile.TemporaryDirectory() as tmp:
            workspace = Path(tmp)
            project = workspace / "analytics"
            (project / "models").mkdir(parents=True)
            (project / "models/model.sql").write_text("select 1 as id\n", encoding="utf-8")
            (project / "target").mkdir()
            (project / "target/manifest.json").write_text('{"nodes": {}}\n', encoding="utf-8")
            git(project, "init")
            git(project, "checkout", "-b", "main")
            git(project, "config", "user.email", "costguard@example.com")
            git(project, "config", "user.name", "Costguard Test")
            git(project, "add", ".")
            git(project, "commit", "-m", "initial")
            git(project, "checkout", "-b", "feature")
            (project / "models/model.sql").write_text("select 2 as id\n", encoding="utf-8")
            completed = run_driver(
                ["run"],
                env={
                    "GITHUB_WORKSPACE": str(workspace),
                    "WORKING_DIRECTORY_INPUT": "analytics",
                    "BASE_INPUT": "main",
                    "WAREHOUSE_INPUT": "generic",
                    "FAIL_ON_INPUT": "high",
                    "FORMAT_INPUT": "json",
                    "PATH": f"{binary_dir}{os.pathsep}{os.environ['PATH']}",
                },
            )
            self.assertEqual(completed.returncode, 0, completed.stderr)
            payload = json.loads(completed.stdout)
            self.assertEqual(payload["schema_version"], 3)
            self.assertEqual(payload["analysis"]["policy"], "standard")
            self.assertTrue(payload["analysis"]["passed"])

    def test_requested_missing_manifest_fails(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            completed = run_driver(
                ["run"],
                env={
                    "GITHUB_WORKSPACE": tmp,
                    "MANIFEST_INPUT": "target/manifest.json",
                },
            )
            self.assertNotEqual(completed.returncode, 0)
            self.assertIn("manifest does not exist", completed.stderr)

    def test_attestation_failure_prevents_extraction(self) -> None:
        target = platform_target()
        asset_name = f"costguard-{target}.tar.gz"
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            bin_dir = root / "bin"
            bin_dir.mkdir()
            fake_gh = bin_dir / "gh"
            fake_gh.write_text("#!/bin/sh\nexit 7\n", encoding="utf-8")
            fake_gh.chmod(0o755)
            (root / asset_name).write_bytes(b"not extracted")
            digest = hashlib.sha256((root / asset_name).read_bytes()).hexdigest()
            (root / f"{asset_name}.sha256").write_text(
                f"{digest}  {asset_name}\n", encoding="utf-8"
            )
            with file_server(root) as base_url:
                completed = run_driver(
                    ["install", "--mode", "release"],
                    env={
                        "COSTGUARD_RELEASE_BASE_URL": base_url,
                        "RUNNER_TEMP": str(root / "runner"),
                        "GITHUB_PATH": str(root / "github-path"),
                        "PATH": f"{bin_dir}{os.pathsep}{os.environ['PATH']}",
                    },
                )
            self.assertNotEqual(completed.returncode, 0)
            self.assertIn("attestation verification failed", completed.stderr)


def platform_target() -> str:
    if sys.platform == "darwin":
        return "aarch64-apple-darwin" if os.uname().machine == "arm64" else "x86_64-apple-darwin"
    if sys.platform.startswith("linux"):
        return "x86_64-unknown-linux-gnu"
    raise unittest.SkipTest(f"unsupported test platform: {sys.platform}")


if __name__ == "__main__":
    unittest.main()
