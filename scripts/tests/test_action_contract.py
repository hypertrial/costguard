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
        self.assertIn("default: \"\"", action.split("dbt-adapter-package:", 1)[1].split("dbt-profile-type:", 1)[0])
        for command in ["install", "plan-compile", "compile", "run"]:
            self.assertIn(f"costguard_action.py\" {command}", action)

    def test_workflows_are_manual_only(self) -> None:
        for path in (ROOT / ".github/workflows").glob("*.yml"):
            text = path.read_text(encoding="utf-8")
            self.assertIn("workflow_dispatch:", text, path.name)
            self.assertNotRegex(text, r"(?m)^  (pull_request|push):")

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
            self.assertIn("\npermissions:\n  contents: read\n", text, path.name)
            self.assertNotIn("contents: write", text, path.name)


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
