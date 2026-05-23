#!/usr/bin/env python3

from __future__ import annotations

import os
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


class PreCommitCostguardTest(unittest.TestCase):
    def test_pre_commit_hooks_yaml_references_script(self) -> None:
        hooks = (ROOT / ".pre-commit-hooks.yaml").read_text(encoding="utf-8")
        self.assertIn("id: costguard-pr", hooks)
        self.assertIn("entry: scripts/pre_commit_costguard.sh", hooks)

    def test_hook_script_is_executable(self) -> None:
        hook = ROOT / "scripts/pre_commit_costguard.sh"
        self.assertTrue(hook.exists())
        self.assertTrue(os.access(hook, os.X_OK))

    def test_hook_skips_outside_git_repo(self) -> None:
        hook = ROOT / "scripts/pre_commit_costguard.sh"
        with tempfile.TemporaryDirectory() as tmp:
            proc = subprocess.run(
                [str(hook)],
                cwd=tmp,
                capture_output=True,
                text=True,
                check=False,
            )
            self.assertEqual(proc.returncode, 0, msg=proc.stdout + proc.stderr)
            self.assertIn("not a git repository", proc.stdout + proc.stderr)


if __name__ == "__main__":
    unittest.main()
