from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
import sys

sys.path.insert(0, str(ROOT / "scripts"))

from calibrate_cost_model import (  # noqa: E402
    build_report,
    interval_contains,
    load_history,
)


class CalibrateCostModelTests(unittest.TestCase):
    def test_interval_contains_actual_near_estimate(self) -> None:
        self.assertTrue(interval_contains(100.0, 100.0))

    def test_build_report_passes_on_tight_history(self) -> None:
        with tempfile.NamedTemporaryFile("w", suffix=".csv", delete=False) as handle:
            handle.write("model_or_table,bytes_per_run,actual_bytes_per_run\n")
            for idx in range(20):
                value = 1e12 * (1.0 + idx * 0.01)
                handle.write(f"m{idx},{value},{value}\n")
            path = Path(handle.name)
        rows = load_history(path)
        report = build_report(rows)
        self.assertGreaterEqual(report["rows"], 20)
        self.assertTrue(report["passes"])


if __name__ == "__main__":
    unittest.main()
