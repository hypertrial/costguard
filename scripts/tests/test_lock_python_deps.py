from __future__ import annotations

import importlib.util
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts" / "lock_python_deps.py"


def load_module():
    spec = importlib.util.spec_from_file_location("lock_python_deps", SCRIPT)
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class PythonLockTests(unittest.TestCase):
    def setUp(self) -> None:
        self.module = load_module()
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.source = self.root / "requirements-test.txt"
        self.lock = self.root / "requirements-test.lock"
        self.source.write_text("example>=1\n", encoding="utf-8")

    def write_valid_lock(self) -> None:
        self.lock.write_text(
            self.module.lock_header(self.source)
            + "example==1.0 \\\n+    --hash=sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n",
            encoding="utf-8",
        )

    def test_valid_lock_passes_offline_validation(self) -> None:
        self.write_valid_lock()
        self.module.validate_lock(self.source, self.lock)

    def test_input_hash_drift_invalidates_lock(self) -> None:
        self.write_valid_lock()
        self.source.write_text("example>=2\n", encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "stale metadata"):
            self.module.validate_lock(self.source, self.lock)

    def test_malformed_header_is_rejected(self) -> None:
        self.lock.write_text("example==1 --hash=sha256:abc\n", encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "malformed"):
            self.module.validate_lock(self.source, self.lock)

    def test_lock_metadata_is_deterministic_and_path_free(self) -> None:
        first = self.module.lock_header(self.source)
        second = self.module.lock_header(self.source)
        self.assertEqual(first, second)
        self.assertNotIn(str(self.root), first)
        self.assertNotIn("generated_at", first)


if __name__ == "__main__":
    unittest.main()
