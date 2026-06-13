from __future__ import annotations

import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SCRIPTS = ROOT / "scripts"
sys.path.insert(0, str(SCRIPTS))

from check_docs import check_internal, markdown_files, slug  # noqa: E402
from costguard_tooling import max_rss_bytes, summarize_measurements  # noqa: E402
from generate_recall_corpus import FIXTURES, write_fixtures  # noqa: E402
from generate_rule_docs import validate_rule_guides  # noqa: E402


class GateScriptTests(unittest.TestCase):
    def test_slug_normalizes_headings(self) -> None:
        self.assertEqual(slug("Quick Start"), "quick-start")

    def test_check_internal_reports_missing_target(self) -> None:
        with tempfile.TemporaryDirectory(dir=ROOT) as tmp:
            source = Path(tmp) / "doc.md"
            source.write_text("[link](missing.md)", encoding="utf-8")
            error = check_internal(source, "missing.md")
            self.assertIsNotNone(error)
            self.assertIn("missing link target", error)

    def test_scale_check_max_rss_bytes(self) -> None:
        self.assertEqual(max_rss_bytes(20_000_000), 20_000_000)
        self.assertEqual(max_rss_bytes(2048), 2048 * 1024)

    def test_measurement_summary_uses_median_max_and_peak_rss(self) -> None:
        summary = summarize_measurements(
            [
                {"runtime_ms": 300, "max_rss_bytes": 30},
                {"runtime_ms": 100, "max_rss_bytes": 10},
                {"runtime_ms": 200, "max_rss_bytes": 20},
            ]
        )
        self.assertEqual(summary["runtime_samples_ms"], [300, 100, 200])
        self.assertEqual(summary["runtime_median_ms"], 200)
        self.assertEqual(summary["runtime_max_ms"], 300)
        self.assertEqual(summary["max_rss_bytes"], 30)

    def test_markdown_files_includes_readme(self) -> None:
        paths = markdown_files()
        self.assertIn(ROOT / "README.md", paths)

    def test_validate_rule_guides_reports_missing_guide(self) -> None:
        errors = validate_rule_guides(
            [{"id": "SQLCOST999", "name": "Missing", "severity": "high"}]
        )
        self.assertTrue(any("missing per-rule guide" in error for error in errors))

    def test_validate_rule_guides_reports_orphan_guide(self) -> None:
        errors = validate_rule_guides(
            [{"id": "SQLCOST001", "name": "Select star", "severity": "high"}]
        )
        self.assertTrue(any("orphan per-rule guide" in error for error in errors))

    def test_generate_recall_corpus_check_passes(self) -> None:
        proc = subprocess.run(
            [sys.executable, str(SCRIPTS / "generate_recall_corpus.py"), "--check"],
            cwd=ROOT,
            capture_output=True,
            text=True,
            check=False,
        )
        self.assertEqual(proc.returncode, 0, proc.stderr or proc.stdout)

    def test_write_fixtures_round_trip(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            corpus = Path(tmp)
            write_fixtures(corpus)
            self.assertTrue((corpus / next(iter(FIXTURES))).exists())

    def test_validate_fp_registry_passes(self) -> None:
        proc = subprocess.run(
            [sys.executable, str(SCRIPTS / "validate_fp_registry.py")],
            cwd=ROOT,
            capture_output=True,
            text=True,
            check=False,
        )
        self.assertEqual(proc.returncode, 0, proc.stderr or proc.stdout)

    def test_generate_rule_docs_check_passes(self) -> None:
        proc = subprocess.run(
            [sys.executable, str(SCRIPTS / "generate_rule_docs.py"), "--check"],
            cwd=ROOT,
            capture_output=True,
            text=True,
            check=False,
        )
        self.assertEqual(proc.returncode, 0, proc.stderr or proc.stdout)


if __name__ == "__main__":
    unittest.main()
