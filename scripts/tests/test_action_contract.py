#!/usr/bin/env python3

from __future__ import annotations

import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
PINNED_ACTION_RE = re.compile(r"^[^@\s]+@[0-9a-f]{40}$")


class ActionContractTest(unittest.TestCase):
    def test_action_uses_driver_and_action_path(self) -> None:
        action = (ROOT / ".github/actions/costguard/action.yml").read_text(encoding="utf-8")
        self.assertIn('${GITHUB_ACTION_PATH}/scripts/costguard_action.py', action)
        self.assertNotIn('${GITHUB_WORKSPACE}/scripts/dbt_compile_for_costguard.py', action)
        for command in ["install", "run"]:
            self.assertIn(f"costguard_action.py\" {command}", action)

    def test_ci_is_automatic_and_release_is_tag_driven(self) -> None:
        ci = (ROOT / ".github/workflows/ci.yml").read_text(encoding="utf-8")
        self.assertRegex(ci, r"(?m)^  pull_request:")
        self.assertRegex(ci, r"(?m)^  push:")
        self.assertIn("branches: [main]", ci)
        self.assertIn("timeout-minutes: 5", ci)
        scale = ci.split("  scale:", 1)[1].split("  spellbook-smoke:", 1)[0]
        self.assertNotIn("github.event_name == 'push'", scale)
        spellbook = ci.split("  spellbook-smoke:", 1)[1].split("  nba-monte-carlo-smoke:", 1)[0]
        nba = ci.split("  nba-monte-carlo-smoke:", 1)[1]
        self.assertIn("github.event_name != 'pull_request'", spellbook)
        self.assertIn("github.event_name != 'pull_request'", nba)

        benchmark = (ROOT / ".github/workflows/benchmark.yml").read_text(
            encoding="utf-8"
        )
        self.assertRegex(benchmark, r"(?m)^  schedule:")
        self.assertIn("workflow_dispatch:", benchmark)

        release = (ROOT / ".github/workflows/release.yml").read_text(
            encoding="utf-8"
        )
        self.assertIn('tags: ["v2.*"]', release)
        self.assertIn("environment: release", release)
        self.assertIn("git verify-tag", release)
        self.assertIn("actions/attest-build-provenance@", release)
        self.assertIn("actions/attest-sbom@", release)
        self.assertIn("--prerelease", release)
        self.assertIn("needs.qualify.outputs.prerelease == 'false'", release)
        self.assertIn("macos-15-intel", release)
        self.assertIn("x86_64-pc-windows-msvc", release)
        self.assertIn("verify_ci_history.py", release)
        self.assertIn("--trust-github-qualification", release)
        self.assertIn("--workflow ci.yml", release)
        self.assertIn("--workflow benchmark.yml", release)
        self.assertIn("--event workflow_dispatch", release)
        self.assertIn("--required-job full-evidence-gate", release)
        self.assertIn("release_consumer_smoke.py", release)
        self.assertIn("timeout-minutes: 5", release)
        build = release.split("  build:", 1)[1].split("  publish:", 1)[0]
        self.assertIn("timeout-minutes: 10", build)

    def test_ci_reuses_cached_tools_and_single_release_build(self) -> None:
        ci = (ROOT / ".github/workflows/ci.yml").read_text(encoding="utf-8")
        ci_local = (ROOT / "scripts/ci_local.sh").read_text(encoding="utf-8")
        pr_gate = ci.split("  pr-gate:", 1)[1].split("  scale:", 1)[0]
        scale = ci.split("  scale:", 1)[1].split("  spellbook-smoke:", 1)[0]
        spellbook = ci.split("  spellbook-smoke:", 1)[1].split("  nba-monte-carlo-smoke:", 1)[0]
        nba = ci.split("  nba-monte-carlo-smoke:", 1)[1]

        self.assertIn("Swatinem/rust-cache@", pr_gate)
        self.assertIn("taiki-e/install-action@", pr_gate)
        self.assertIn("tool: mdbook@0.4.40,cargo-deny@0.19.7", pr_gate)
        self.assertIn('CARGO_INCREMENTAL: "0"', pr_gate)
        self.assertIn("name: ci-release-binary", pr_gate)
        self.assertEqual(ci.count("cargo build --release --locked -p costguard-cli"), 0)
        self.assertEqual(
            ci_local.count("cargo build --release --locked -p costguard-cli"), 1
        )
        self.assertNotIn("cargo build --locked -p costguard-cli", ci_local)
        for job in [scale, spellbook, nba]:
            self.assertIn("actions/download-artifact@", job)
            self.assertIn("name: ci-release-binary", job)
            self.assertIn("chmod +x target/release/costguard", job)

    def test_action_run_blocks_do_not_interpolate_inputs_directly(self) -> None:
        action = (ROOT / ".github/actions/costguard/action.yml").read_text(encoding="utf-8")
        for block in run_blocks(action):
            self.assertNotIn("${{ inputs.", block)

    def test_external_actions_are_pinned_to_full_commit_shas(self) -> None:
        for path in (ROOT / ".github").rglob("*.yml"):
            text = path.read_text(encoding="utf-8")
            for line in text.splitlines():
                match = re.search(r"\buses:\s+([^\s#]+)", line)
                if match is None:
                    continue
                action_ref = match.group(1)
                if action_ref.startswith("./"):
                    continue
                self.assertRegex(
                    action_ref,
                    PINNED_ACTION_RE,
                    msg=f"{path.relative_to(ROOT)} uses unpinned action {action_ref}",
                )

    def test_workflows_use_read_only_permissions(self) -> None:
        for path in (ROOT / ".github/workflows").glob("*.yml"):
            text = path.read_text(encoding="utf-8")
            workflow_permissions = text.split("\njobs:\n", 1)[0]
            self.assertIn("\npermissions:\n", workflow_permissions, path.name)
            self.assertIn("  contents: read\n", workflow_permissions, path.name)
            if path.name != "release.yml":
                self.assertNotIn("contents: write", text, path.name)

    def test_action_defaults_are_manifest_first_and_standard(self) -> None:
        action = (ROOT / ".github/actions/costguard/action.yml").read_text(encoding="utf-8")
        self.assertNotIn("compile-dbt:", action)
        self.assertNotIn("server-url:", action)
        self.assertNotIn("publication-mode:", action)
        analysis_block = action.split("analysis-policy:", 1)[1].split(
            "verify-attestation:", 1
        )[0]
        self.assertIn("default: standard", analysis_block)
        self.assertIn("verify-attestation:", action)
        self.assertIn("manifest:", action)
        self.assertIn("baseline:", action)
        block_only_new = action.split("block-only-new:", 1)[1].split(
            "fail-on-pr-cost-increase:", 1
        )[0]
        self.assertIn('default: "true"', block_only_new)
        self.assertIn("FAIL_ON_PR_COST_INCREASE_INPUT", action)
        for policy_input in [
            "policy:",
            "trust-store:",
            "policy-organization:",
            "policy-team:",
            "policy-repository:",
            "receipt-path:",
            "compare-receipt:",
            "pr-comment:",
            "github-token:",
        ]:
            self.assertIn(policy_input, action)
        comment_block = action.split("pr-comment:", 1)[1].split("github-token:", 1)[0]
        self.assertIn('default: "false"', comment_block)
        self.assertIn("PR_COMMENT_INPUT", action)
        self.assertIn("GITHUB_TOKEN_INPUT", action)


def run_blocks(text: str) -> list[str]:
    lines = text.splitlines()
    blocks: list[str] = []
    for index, line in enumerate(lines):
        if not re.match(r"^\s*run:\s*\|", line):
            continue
        indent = len(line) - len(line.lstrip(" "))
        block_lines = []
        for next_line in lines[index + 1 :]:
            next_indent = len(next_line) - len(next_line.lstrip(" "))
            if next_line.strip() and next_indent <= indent:
                break
            block_lines.append(next_line)
        blocks.append("\n".join(block_lines))
    return blocks


if __name__ == "__main__":
    unittest.main()
