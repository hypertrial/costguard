# Cost model design

Costguard estimates monthly spend per finding using a **lognormal product model**. All inputs are local files; the scanner never connects to a warehouse.

## Formula

```
monthly_cost ≈ base_bytes × rule_multiplier × runs_per_month × price_per_byte
```

Each factor is modeled as a lognormal distribution in log-space. The product of independent lognormals is lognormal with:

- `μ_product = Σ μ_i`
- `σ²_product = Σ σ²_i`

This yields closed-form p10/p50/p90 quantiles without Monte Carlo simulation.

## Factor definitions

### Base bytes

Resolved per dbt model in priority order:

1. Query-history CSV (`bytes_per_run`) — grade **A**
2. `catalog.json` `stats.num_bytes` — grade **B**
3. `[cost.sources]` bytes/rows — grade **B**
4. `default_table_size` prior — grade **C**

Incremental models with a valid `unique_key` scale base bytes by `incremental_fraction` (default 5%).

### Rule multiplier

Default lognormal priors per `SQLCOST*` rule (e.g. missing partition 5–100×, cross join 5–50×, repeated CTE 2–6×). Overridable via `[cost.rules.RULE_ID].multiplier`.

Infrastructure rules `SQLCOST023`–`SQLCOST027` receive no cost estimate.

### Runs per month

`[cost.models]` override, else `[cost].default_runs_per_month` (default 30).

### Pricing

| Regime | Config | Uncertainty |
| --- | --- | --- |
| Scan | `model = "scan"`, `usd_per_tb` | Low (CV ~5%) |
| Compute | `model = "compute"`, `usd_per_credit`, `tb_per_credit_hour` | High unless calibrated |

## Confidence grades

| Grade | Meaning |
| --- | --- |
| A | Measured from offline query-history export |
| B | Catalog stats or explicit `[cost.sources]` config |
| C | Default size-class prior only |

Text output always shows the grade so readers know how much to trust the interval width.

## Gating

`fail_on_monthly_delta` fails the check when the sum of **p50** across post-baseline findings exceeds the threshold. This composes with severity-based `--fail-on` (either condition fails).

Cost gating is **optional enrichment**, not a replacement for severity gates. See [PR check workflow](pr-check-primary-workflow.md).

## Calibration

`scripts/calibrate_cost_model.py` reads a query-history CSV and:

1. Computes 80% interval coverage (actual vs estimated bytes per run).
2. Suggests `tb_per_credit_hour` bounds for compute-priced repos.
3. Fails when coverage is outside 60–95% (intervals too wide or overconfident).

## Implementation

- Rust crate: `costguard-cost` (lognormal math, volume resolver, annotation pass)
- Annotation runs after baseline filtering in `costguard-core/src/scan.rs`
- Baseline fingerprints ignore cost fields (`rule_id|path|message`)

## Limitations

- Estimates rank findings and gate PR cost deltas at order-of-magnitude fidelity; they are not a query optimizer or billing system of record.
- Cross-file rules use the diagnosed model's volume, not full-project aggregation.
- Compute-priced intervals remain wide until query-history calibration narrows `tb_per_credit_hour`.
