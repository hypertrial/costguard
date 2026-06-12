#!/usr/bin/env python3
"""Calibrate Costguard cost-model parameters from offline query-history exports."""

from __future__ import annotations

import argparse
import csv
import json
import math
import sys
from pathlib import Path
from typing import Any

DEFAULT_INTERVAL = 0.80
COVERAGE_MIN = 0.60
COVERAGE_MAX = 0.95


def normal_quantile(p: float) -> float:
    if p <= 0.0:
        return float("-inf")
    if p >= 1.0:
        return float("inf")
    a = [
        -3.969683028665376e01,
        2.209460984245205e02,
        -2.759285469016765e02,
        1.383577518672690e02,
        -3.066479806614716e01,
        2.506628277459239e00,
    ]
    b = [
        -5.447609879822406e01,
        1.615858368580409e02,
        -1.556989775598873e02,
        6.680131188771972e01,
        -1.328068155288572e01,
    ]
    c = [
        -7.784894002430293e-03,
        -3.223964580411865e-01,
        -2.400758277161838e+00,
        -2.549732539343734e+00,
        4.374664141464968e+00,
        2.938163982698783e+00,
    ]
    d = [
        7.784695709041462e-03,
        3.224671290700398e-01,
        2.445134137142996e+00,
        3.754408661907416e+00,
    ]
    p_low = 0.02425
    p_high = 1.0 - p_low
    if p < p_low:
        q = math.sqrt(-2.0 * math.log(p))
        num = (((((c[0] * q + c[1]) * q + c[2]) * q + c[3]) * q + c[4]) * q + c[5])
        den = ((((d[0] * q + d[1]) * q + d[2]) * q + d[3]) * q + 1.0)
        return num / den
    if p > p_high:
        q = math.sqrt(-2.0 * math.log(1.0 - p))
        num = (((((c[0] * q + c[1]) * q + c[2]) * q + c[3]) * q + c[4]) * q + c[5])
        den = ((((d[0] * q + d[1]) * q + d[2]) * q + d[3]) * q + 1.0)
        return -num / den
    q = p - 0.5
    r = q * q
    num = (((((a[0] * r + a[1]) * r + a[2]) * r + a[3]) * r + a[4]) * r + a[5]) * q
    den = (((((b[0] * r + b[1]) * r + b[2]) * r + b[3]) * r + b[4]) * r + 1.0)
    return num / den


def load_history(path: Path) -> list[dict[str, float | str]]:
    rows: list[dict[str, float | str]] = []
    with path.open(newline="", encoding="utf-8") as handle:
        reader = csv.DictReader(handle)
        for row in reader:
            key = row.get("model_or_table") or row.get("model") or row.get("table")
            if not key:
                continue
            bytes_per_run = float(row.get("bytes_per_run") or row.get("bytes_billed") or 0)
            actual = float(row.get("actual_bytes_per_run") or bytes_per_run)
            rows.append(
                {
                    "model_or_table": key.strip(),
                    "bytes_per_run": bytes_per_run,
                    "actual_bytes_per_run": actual,
                }
            )
    return rows


def fit_lognormal(p10: float, p90: float) -> tuple[float, float]:
    z = normal_quantile(0.9) - normal_quantile(0.1)
    sigma = (math.log(p90) - math.log(p10)) / z
    mu = (math.log(p10) + math.log(p90)) / 2.0
    return mu, sigma


def interval_contains(actual: float, estimate: float, cv: float = 0.5) -> bool:
    mu = math.log(max(estimate, 1.0))
    sigma = math.sqrt(math.log(1.0 + cv * cv))
    tail = (1.0 - DEFAULT_INTERVAL) / 2.0
    lo = math.exp(mu + sigma * normal_quantile(tail))
    hi = math.exp(mu + sigma * normal_quantile(1.0 - tail))
    return lo <= actual <= hi


def build_report(rows: list[dict[str, float | str]]) -> dict[str, Any]:
    if not rows:
        return {"coverage": None, "rows": 0, "passes": False}
    hits = sum(
        1
        for row in rows
        if interval_contains(
            float(row["actual_bytes_per_run"]),
            float(row["bytes_per_run"]),
        )
    )
    coverage = hits / len(rows)
    estimates = sorted(float(row["bytes_per_run"]) for row in rows)
    actuals = sorted(float(row["actual_bytes_per_run"]) for row in rows)
    p10_e = estimates[max(0, int(0.1 * len(estimates)) - 1)]
    p90_e = estimates[min(len(estimates) - 1, int(0.9 * len(estimates)))]
    p10_a = actuals[max(0, int(0.1 * len(actuals)) - 1)]
    p90_a = actuals[min(len(actuals) - 1, int(0.9 * len(actuals)))]
    mu_e, sigma_e = fit_lognormal(max(p10_e, 1.0), max(p90_e, p10_e + 1.0))
    mu_a, _sigma_a = fit_lognormal(max(p10_a, 1.0), max(p90_a, p10_a + 1.0))
    tb_per_credit_hour = {
        "p10": round(math.exp(mu_a - mu_e - sigma_e) / 1e12, 3),
        "p90": round(math.exp(mu_a - mu_e + sigma_e) / 1e12, 3),
    }
    passes = coverage >= COVERAGE_MIN
    too_wide = coverage > COVERAGE_MAX
    return {
        "rows": len(rows),
        "coverage": round(coverage, 3),
        "coverage_band": [COVERAGE_MIN, COVERAGE_MAX],
        "passes": passes,
        "too_wide": too_wide,
        "tb_per_credit_hour_suggestion": tb_per_credit_hour,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("history", type=Path, help="Query-history CSV export")
    parser.add_argument("--json", action="store_true", help="Emit JSON report")
    args = parser.parse_args()
    rows = load_history(args.history)
    report = build_report(rows)
    if args.json:
        print(json.dumps(report, indent=2))
    else:
        print(f"Rows: {report['rows']}")
        print(f"80% interval coverage: {report['coverage']}")
        print(f"Suggested tb_per_credit_hour: {report['tb_per_credit_hour_suggestion']}")
        if report.get("too_wide"):
            print("WARN: intervals may be too wide (>95% coverage)")
        print("PASS" if report["passes"] else "FAIL")
    return 0 if report["passes"] else 1


if __name__ == "__main__":
    sys.exit(main())
