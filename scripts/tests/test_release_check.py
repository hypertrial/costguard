from __future__ import annotations

import subprocess
import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


class ReleaseCheckTest(unittest.TestCase):
    def test_skip_flags_cannot_create_release_evidence(self) -> None:
        completed = subprocess.run(
            [
                sys.executable,
                str(ROOT / "scripts/release_check.py"),
                "--version",
                "2.7.0",
                "--skip-external",
            ],
            cwd=ROOT,
            capture_output=True,
            text=True,
            check=False,
        )
        self.assertNotEqual(completed.returncode, 0)
        self.assertIn("release evidence cannot be created with skip flags", completed.stderr)

    def test_trust_push_ci_skips_heavy_gate(self) -> None:
        completed = subprocess.run(
            [
                sys.executable,
                str(ROOT / "scripts/release_check.py"),
                "--version",
                "2.7.0",
                "--trust-push-ci",
                "--development",
            ],
            cwd=ROOT,
            capture_output=True,
            text=True,
            check=False,
        )
        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertIn("trust-push-ci", completed.stdout)

    def test_trust_github_qualification_skips_heavy_gate(self) -> None:
        completed = subprocess.run(
            [
                sys.executable,
                str(ROOT / "scripts/release_check.py"),
                "--version",
                "2.7.0",
                "--trust-github-qualification",
                "--development",
            ],
            cwd=ROOT,
            capture_output=True,
            text=True,
            check=False,
        )
        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertIn("trust-github-qualification", completed.stdout)


if __name__ == "__main__":
    unittest.main()
