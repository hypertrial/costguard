#!/usr/bin/env python3

from __future__ import annotations

import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
PINNED_ACTION_RE = re.compile(r"^[^@\s]+@[0-9a-f]{40}$")


class ActionContractTest(unittest.TestCase):
    def test_action_exposes_enterprise_install_and_compile_inputs(self) -> None:
        action = (ROOT / ".github/actions/costguard/action.yml").read_text(encoding="utf-8")
        for expected in [
            "install-mode:",
            "version:",
            "dbt-requirements-file:",
            "dbt-constraints-file:",
            "dbt-vars:",
            "fail-on-deps-failure:",
            "use-existing-manifest:",
            "Install costguard release",
            "Build costguard CLI",
        ]:
            self.assertIn(expected, action)

    def test_repo_pr_workflow_uses_source_install_mode(self) -> None:
        workflow = (ROOT / ".github/workflows/costguard-pr.yml").read_text(encoding="utf-8")
        self.assertIn("install-mode: source", workflow)

    def test_release_action_and_workflow_share_asset_contract(self) -> None:
        action = (ROOT / ".github/actions/costguard/action.yml").read_text(encoding="utf-8")
        release = (ROOT / ".github/workflows/release.yml").read_text(encoding="utf-8")
        self.assertIn('asset="costguard-${target}.tar.gz"', action)
        self.assertIn('asset="costguard-${{ matrix.target }}.tar.gz"', release)
        self.assertIn("releases/download/${version}", action)
        self.assertIn('.sha256"', action)

    def test_action_run_blocks_do_not_interpolate_inputs_directly(self) -> None:
        action = (ROOT / ".github/actions/costguard/action.yml").read_text(encoding="utf-8")
        for block in run_blocks(action):
            self.assertNotIn("${{ inputs.", block)

    def test_costguard_pr_step_uses_env_for_shell_inputs(self) -> None:
        action = (ROOT / ".github/actions/costguard/action.yml").read_text(encoding="utf-8")
        step = action.split("- name: Run costguard pr", 1)[1]
        for expected in [
            "BASE_INPUT: ${{ inputs.base }}",
            "WAREHOUSE_INPUT: ${{ inputs.warehouse }}",
            "FAIL_ON_INPUT: ${{ inputs.fail-on }}",
            "FORMAT_INPUT: ${{ inputs.format }}",
            "MIN_CONFIDENCE_INPUT: ${{ inputs.min-confidence }}",
            "MANIFEST_INPUT: ${{ inputs.manifest }}",
        ]:
            self.assertIn(expected, step)
        for expected in [
            '"${BASE_INPUT}"',
            '"${WAREHOUSE_INPUT}"',
            '"${FAIL_ON_INPUT}"',
            '"${FORMAT_INPUT}"',
            '"${MIN_CONFIDENCE_INPUT}"',
            '"${MANIFEST_INPUT}"',
            '"${FORMAT_INPUT}" = "markdown"',
        ]:
            self.assertIn(expected, step)

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

    def test_workflows_use_read_only_default_permissions(self) -> None:
        for path in (ROOT / ".github/workflows").glob("*.yml"):
            text = path.read_text(encoding="utf-8")
            self.assertIn("\npermissions:\n  contents: read\n", text, path.name)
            if path.name != "release.yml":
                self.assertNotIn("contents: write", text, path.name)

        release = (ROOT / ".github/workflows/release.yml").read_text(encoding="utf-8")
        self.assertIn(
            "  publish:\n"
            "    needs: build\n"
            "    runs-on: ubuntu-latest\n"
            "    permissions:\n"
            "      contents: write\n",
            release,
        )


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
