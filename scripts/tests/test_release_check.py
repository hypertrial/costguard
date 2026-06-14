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
                "2.1.0",
                "--skip-external",
            ],
            cwd=ROOT,
            capture_output=True,
            text=True,
            check=False,
        )
        self.assertNotEqual(completed.returncode, 0)
        self.assertIn("release evidence cannot be created with skip flags", completed.stderr)


if __name__ == "__main__":
    unittest.main()
