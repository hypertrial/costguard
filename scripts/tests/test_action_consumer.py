#!/usr/bin/env python3

from __future__ import annotations

import contextlib
import functools
import hashlib
import http.server
import importlib.util
import io
import json
import os
import subprocess
import sys
import tarfile
import tempfile
import threading
import unittest
import urllib.parse
from pathlib import Path
from unittest import mock
from urllib.error import URLError

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


@contextlib.contextmanager
def comment_server(comments: list[dict[str, object]] | None = None, reject: bool = False):
    class Handler(http.server.BaseHTTPRequestHandler):
        def log_message(self, _format: str, *_args: object) -> None:
            return

        def respond(self, status: int, payload: object) -> None:
            body = json.dumps(payload).encode("utf-8")
            self.send_response(status)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def do_GET(self) -> None:
            self.server.auth_headers.append(self.headers.get("Authorization"))
            if self.server.reject:
                self.respond(403, {"message": "forbidden"})
                return
            query = urllib.parse.parse_qs(urllib.parse.urlparse(self.path).query)
            page = int(query.get("page", ["1"])[0])
            start = (page - 1) * 100
            self.respond(200, self.server.comments[start : start + 100])

        def do_POST(self) -> None:
            self.server.auth_headers.append(self.headers.get("Authorization"))
            payload = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
            comment = {
                "id": 1001,
                "body": payload["body"],
                "user": {"login": "github-actions[bot]", "type": "Bot"},
            }
            self.server.comments.append(comment)
            self.respond(201, comment)

        def do_PATCH(self) -> None:
            self.server.auth_headers.append(self.headers.get("Authorization"))
            payload = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
            identifier = int(self.path.rsplit("/", 1)[1])
            comment = next(item for item in self.server.comments if item["id"] == identifier)
            comment["body"] = payload["body"]
            self.respond(200, comment)

    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    server.comments = list(comments or [])
    server.auth_headers = []
    server.reject = reject
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        yield server, f"http://127.0.0.1:{server.server_port}"
    finally:
        server.shutdown()
        thread.join()
        server.server_close()


def git(root: Path, *args: str) -> None:
    subprocess.run(["git", *args], cwd=root, check=True, capture_output=True, text=True)


class ActionConsumerTest(unittest.TestCase):
    def test_sticky_comment_creates_then_updates_without_duplicates(self) -> None:
        driver = load_driver_module()
        with tempfile.TemporaryDirectory() as tmp, comment_server() as (server, api):
            event = Path(tmp) / "event.json"
            event.write_text('{"pull_request":{"number":7}}', encoding="utf-8")
            environment = {
                "GITHUB_TOKEN_INPUT": "secret-token",
                "GITHUB_REPOSITORY": "acme/warehouse",
                "GITHUB_API_URL": api,
                "GITHUB_EVENT_PATH": str(event),
            }
            with mock.patch.dict(os.environ, environment, clear=False):
                driver.publish_pr_comment("# First")
                driver.publish_pr_comment("# Updated")
            self.assertEqual(len(server.comments), 1)
            self.assertIn(driver.PR_COMMENT_MARKER, server.comments[0]["body"])
            self.assertIn("# Updated", server.comments[0]["body"])
            self.assertNotIn("# First", server.comments[0]["body"])
            self.assertTrue(server.auth_headers)
            self.assertTrue(all(value == "Bearer secret-token" for value in server.auth_headers))

    def test_sticky_comment_finds_marker_on_later_page(self) -> None:
        driver = load_driver_module()
        comments = [
            {"id": index, "body": "other", "user": {"login": "bot[bot]", "type": "Bot"}}
            for index in range(1, 101)
        ]
        comments.append(
            {
                "id": 501,
                "body": driver.PR_COMMENT_MARKER,
                "user": {"login": "github-actions[bot]", "type": "Bot"},
            }
        )
        with tempfile.TemporaryDirectory() as tmp, comment_server(comments) as (server, api):
            event = Path(tmp) / "event.json"
            event.write_text('{"pull_request":{"number":7}}', encoding="utf-8")
            with mock.patch.dict(
                os.environ,
                {
                    "GITHUB_TOKEN_INPUT": "token",
                    "GITHUB_REPOSITORY": "acme/warehouse",
                    "GITHUB_API_URL": api,
                    "GITHUB_EVENT_PATH": str(event),
                },
                clear=False,
            ):
                driver.publish_pr_comment("# Page two")
            self.assertEqual(len(server.comments), 101)
            self.assertIn("# Page two", server.comments[-1]["body"])

    def test_comment_http_403_is_advisory(self) -> None:
        driver = load_driver_module()
        with tempfile.TemporaryDirectory() as tmp, comment_server(reject=True) as (_server, api):
            event = Path(tmp) / "event.json"
            event.write_text('{"pull_request":{"number":7}}', encoding="utf-8")
            stderr = io.StringIO()
            with mock.patch.dict(
                os.environ,
                {
                    "GITHUB_TOKEN_INPUT": "never-log-this",
                    "GITHUB_REPOSITORY": "acme/warehouse",
                    "GITHUB_API_URL": api,
                    "GITHUB_EVENT_PATH": str(event),
                },
                clear=False,
            ), contextlib.redirect_stderr(stderr):
                driver.publish_pr_comment("# Summary")
            self.assertIn("HTTP 403", stderr.getvalue())
            self.assertNotIn("never-log-this", stderr.getvalue())

    def test_comment_missing_token_is_advisory(self) -> None:
        driver = load_driver_module()
        stderr = io.StringIO()
        with mock.patch.dict(os.environ, {"GITHUB_TOKEN_INPUT": ""}, clear=False), (
            contextlib.redirect_stderr(stderr)
        ):
            driver.publish_pr_comment("# Summary")
        self.assertIn("github-token is empty", stderr.getvalue())

    def test_comment_missing_pull_request_context_is_advisory(self) -> None:
        driver = load_driver_module()
        stderr = io.StringIO()
        with mock.patch.dict(
            os.environ,
            {
                "GITHUB_TOKEN_INPUT": "never-log-this",
                "GITHUB_REPOSITORY": "acme/warehouse",
                "GITHUB_EVENT_PATH": "",
            },
            clear=False,
        ), contextlib.redirect_stderr(stderr):
            driver.publish_pr_comment("# Summary")
        self.assertIn("context is unavailable", stderr.getvalue())
        self.assertNotIn("never-log-this", stderr.getvalue())

    def test_floating_major_action_uses_exact_workspace_release(self) -> None:
        self.assertEqual(load_driver_module().action_release_version(), "v2.6.0")

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
                    ["install", "--mode", "release", "--version", "v2.6.0"],
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

    def test_release_install_rejects_checksum_for_another_asset(self) -> None:
        target = platform_target()
        asset_name = f"costguard-{target}.tar.gz"
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / asset_name).write_bytes(b"archive")
            digest = hashlib.sha256(b"archive").hexdigest()
            (root / f"{asset_name}.sha256").write_text(
                f"{digest}  another-asset.tar.gz\n", encoding="utf-8"
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
            self.assertIn("invalid checksum file", completed.stderr)

    def test_release_install_rejects_malformed_checksum_digest(self) -> None:
        target = platform_target()
        asset_name = f"costguard-{target}.tar.gz"
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / asset_name).write_bytes(b"archive")
            (root / f"{asset_name}.sha256").write_text(
                f"not-a-sha256  {asset_name}\n", encoding="utf-8"
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
            self.assertIn("invalid checksum file", completed.stderr)

    def test_release_install_rejects_unexpected_archive_layout(self) -> None:
        target = platform_target()
        asset_name = f"costguard-{target}.tar.gz"
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            asset = root / asset_name
            unexpected = root / "unexpected"
            unexpected.write_text("bad", encoding="utf-8")
            with tarfile.open(asset, "w:gz") as archive:
                archive.add(unexpected, arcname="unexpected")
            digest = hashlib.sha256(asset.read_bytes()).hexdigest()
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
                        "VERIFY_ATTESTATION_INPUT": "false",
                    },
                )
            self.assertNotEqual(completed.returncode, 0)
            self.assertIn("unexpected archive layout", completed.stderr)

    def test_release_install_rejects_link_named_as_binary(self) -> None:
        target = platform_target()
        asset_name = f"costguard-{target}.tar.gz"
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            asset = root / asset_name
            link = tarfile.TarInfo("costguard")
            link.type = tarfile.SYMTYPE
            link.linkname = "/tmp/not-costguard"
            with tarfile.open(asset, "w:gz") as archive:
                archive.addfile(link)
            digest = hashlib.sha256(asset.read_bytes()).hexdigest()
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
                        "VERIFY_ATTESTATION_INPUT": "false",
                    },
                )
            self.assertNotEqual(completed.returncode, 0)
            self.assertIn("unexpected archive layout", completed.stderr)

    def test_download_retries_with_bounded_timeout(self) -> None:
        driver = load_driver_module()
        with tempfile.TemporaryDirectory() as tmp:
            destination = Path(tmp) / "asset"
            with (
                mock.patch.object(
                    driver.urllib.request,
                    "urlopen",
                    side_effect=[URLError("one"), URLError("two"), io.BytesIO(b"ok")],
                ) as urlopen,
                mock.patch.object(driver.time, "sleep") as sleep,
            ):
                driver.download("https://example.invalid/asset", destination, 1024)
            self.assertEqual(destination.read_bytes(), b"ok")
            self.assertEqual(urlopen.call_count, 3)
            self.assertEqual(
                urlopen.call_args.kwargs["timeout"], driver.DOWNLOAD_TIMEOUT_SECONDS
            )
            self.assertEqual(sleep.call_count, 2)

    def test_download_rejects_oversized_content_length_without_retry(self) -> None:
        driver = load_driver_module()
        response = io.BytesIO(b"ignored")
        response.headers = {"Content-Length": "5"}
        with tempfile.TemporaryDirectory() as tmp:
            destination = Path(tmp) / "asset"
            with mock.patch.object(
                driver.urllib.request, "urlopen", return_value=response
            ) as urlopen:
                with self.assertRaisesRegex(SystemExit, "5 bytes.*4 bytes"):
                    driver.download("https://example.invalid/asset", destination, 4)
            self.assertEqual(urlopen.call_count, 1)
            self.assertFalse(destination.exists())

    def test_download_rejects_oversized_stream_and_removes_partial_file(self) -> None:
        driver = load_driver_module()
        response = io.BytesIO(b"12345")
        response.headers = {"Content-Length": "1"}
        with tempfile.TemporaryDirectory() as tmp:
            destination = Path(tmp) / "asset"
            with mock.patch.object(
                driver.urllib.request, "urlopen", return_value=response
            ) as urlopen:
                with self.assertRaisesRegex(SystemExit, "streaming"):
                    driver.download("https://example.invalid/asset", destination, 4)
            self.assertEqual(urlopen.call_count, 1)
            self.assertFalse(destination.exists())

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
            self.assertEqual(payload["schema_version"], 4)
            self.assertEqual(payload["analysis"]["policy"], "standard")
            self.assertTrue(payload["analysis"]["passed"])

    def test_run_passes_only_configured_policy_inputs(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            workspace = Path(tmp)
            bin_dir = workspace / "bin"
            bin_dir.mkdir()
            args_path = workspace / "args.txt"
            fake = bin_dir / "costguard"
            fake.write_text(
                "#!/bin/sh\n"
                f"printf '%s\\n' \"$@\" > '{args_path}'\n"
                "printf '%s\\n' '{\"schema_version\":4,\"analysis\":{\"passed\":true}}'\n",
                encoding="utf-8",
            )
            fake.chmod(0o755)
            completed = run_driver(
                ["run"],
                env={
                    "GITHUB_WORKSPACE": str(workspace),
                    "POLICY_BUNDLE_INPUT": "policy.signed.json",
                    "TRUST_STORE_INPUT": ".costguard/trust.json",
                    "POLICY_ORGANIZATION_INPUT": "acme",
                    "POLICY_REPOSITORY_INPUT": "acme/warehouse",
                    "BLOCK_ONLY_NEW_INPUT": "false",
                    "FAIL_ON_PR_COST_INCREASE_INPUT": "250",
                    "ROCKY_ARTIFACT_INPUT": "target/costguard-rocky.json",
                    "BASE_ROCKY_ARTIFACT_INPUT": "artifacts/base-rocky.json",
                    "PATH": f"{bin_dir}{os.pathsep}{os.environ['PATH']}",
                },
            )
            self.assertEqual(completed.returncode, 0, completed.stderr)
            arguments = args_path.read_text(encoding="utf-8").splitlines()
            self.assertIn("--policy", arguments)
            self.assertIn("policy.signed.json", arguments)
            self.assertIn("--trust-store", arguments)
            self.assertIn("--policy-organization", arguments)
            self.assertIn("--policy-repository", arguments)
            self.assertNotIn("--policy-team", arguments)
            self.assertIn("--block-only-new=false", arguments)
            self.assertIn("--fail-on-pr-cost-increase", arguments)
            self.assertIn("250", arguments)
            self.assertIn("--rocky-artifact", arguments)
            self.assertIn("target/costguard-rocky.json", arguments)
            self.assertIn("--base-rocky-artifact", arguments)
            self.assertIn("artifacts/base-rocky.json", arguments)

    def test_run_writes_step_summary_and_forwards_receipt_inputs(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            workspace = Path(tmp)
            bin_dir = workspace / "bin"
            bin_dir.mkdir()
            args_path = workspace / "args.txt"
            step_summary = workspace / "step-summary.md"
            fake = bin_dir / "costguard"
            fake.write_text(
                "#!/bin/sh\n"
                f"printf '%s\\n' \"$@\" > '{args_path}'\n"
                "while [ \"$#\" -gt 0 ]; do\n"
                "  if [ \"$1\" = '--summary-file' ]; then\n"
                "    shift\n"
                "    printf '# Costguard passed\\n' > \"$1\"\n"
                "  fi\n"
                "  shift\n"
                "done\n"
                "printf 'github annotations\\n'\n",
                encoding="utf-8",
            )
            fake.chmod(0o755)
            completed = run_driver(
                ["run"],
                env={
                    "GITHUB_WORKSPACE": str(workspace),
                    "GITHUB_STEP_SUMMARY": str(step_summary),
                    "RUNNER_TEMP": str(workspace / "runner"),
                    "RECEIPT_PATH_INPUT": "costguard-receipt.json",
                    "COMPARE_RECEIPT_INPUT": "previous.json",
                    "PATH": f"{bin_dir}{os.pathsep}{os.environ['PATH']}",
                },
            )
            self.assertEqual(completed.returncode, 0, completed.stderr)
            self.assertEqual(completed.stdout, "github annotations\n")
            self.assertIn("# Costguard passed", step_summary.read_text(encoding="utf-8"))
            arguments = args_path.read_text(encoding="utf-8").splitlines()
            self.assertIn("--block-only-new=true", arguments)
            self.assertIn("--summary-file", arguments)
            self.assertIn("--receipt-file", arguments)
            self.assertIn("costguard-receipt.json", arguments)
            self.assertIn("--compare-receipt", arguments)
            self.assertIn("previous.json", arguments)

    def test_failed_scan_still_publishes_comment_and_preserves_exit(self) -> None:
        with tempfile.TemporaryDirectory() as tmp, comment_server() as (server, api):
            workspace = Path(tmp)
            bin_dir = workspace / "bin"
            bin_dir.mkdir()
            fake = bin_dir / "costguard"
            fake.write_text(
                "#!/bin/sh\n"
                "while [ \"$#\" -gt 0 ]; do\n"
                "  if [ \"$1\" = '--summary-file' ]; then\n"
                "    shift\n"
                "    printf '# Costguard failed this PR\\n' > \"$1\"\n"
                "  fi\n"
                "  shift\n"
                "done\n"
                "exit 1\n",
                encoding="utf-8",
            )
            fake.chmod(0o755)
            event = workspace / "event.json"
            event.write_text('{"pull_request":{"number":9}}', encoding="utf-8")
            completed = run_driver(
                ["run"],
                env={
                    "GITHUB_WORKSPACE": str(workspace),
                    "RUNNER_TEMP": str(workspace / "runner"),
                    "PR_COMMENT_INPUT": "true",
                    "GITHUB_TOKEN_INPUT": "token",
                    "GITHUB_REPOSITORY": "acme/warehouse",
                    "GITHUB_API_URL": api,
                    "GITHUB_EVENT_PATH": str(event),
                    "PATH": f"{bin_dir}{os.pathsep}{os.environ['PATH']}",
                },
            )
            self.assertEqual(completed.returncode, 1, completed.stderr)
            self.assertEqual(len(server.comments), 1)
            self.assertIn("# Costguard failed this PR", server.comments[0]["body"])

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

    def test_attestation_is_bound_to_producer_repository(self) -> None:
        driver = load_driver_module()
        completed = subprocess.CompletedProcess([], 0, "", "")
        with (
            mock.patch.object(driver.shutil, "which", return_value="/usr/bin/gh"),
            mock.patch.object(driver.subprocess, "run", return_value=completed) as run,
            mock.patch.dict(os.environ, {"GITHUB_REPOSITORY": "consumer/example"}),
        ):
            driver.verify_attestation(Path("asset.tar.gz"))
        self.assertEqual(
            run.call_args.args[0],
            [
                "/usr/bin/gh",
                "attestation",
                "verify",
                "asset.tar.gz",
                "--repo",
                "hypertrial/costguard",
            ],
        )


def platform_target() -> str:
    if sys.platform == "darwin":
        return "aarch64-apple-darwin" if os.uname().machine == "arm64" else "x86_64-apple-darwin"
    if sys.platform.startswith("linux"):
        return "x86_64-unknown-linux-gnu"
    raise unittest.SkipTest(f"unsupported test platform: {sys.platform}")


if __name__ == "__main__":
    unittest.main()
