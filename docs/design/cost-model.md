# Cost model design (v2)

Costguard estimates monthly spend using a **two-stage, model-centric** lognormal product model. All inputs are local files; the scanner never connects to a warehouse.

## Stage 1 — Per-model monthly cost

For every dbt model in the project (not only models with findings):

```
model_monthly_scan = base_bytes × runs_per_month
model_monthly_cost = model_monthly_scan × price_per_byte   (when pricing configured)
```

Base bytes resolve in priority order:

1. Normalized observation bundle (`[cost.inputs].observations`) — grade **A** (USD > credits > bytes; no partition/view/incremental priors)
2. Query-history CSV (`bytes_per_run`) — grade **A** (no priors on measured data)
3. `catalog.json` `stats.num_bytes` — grade **B**
3. `catalog.json` `stats.row_count × avg_row_bytes` (default 200) — grade **B**
4. `[cost.sources]` bytes/rows — grade **B**
5. `default_table_size` prior — grade **C**

Adjustments before pricing (catalog, config source, and size prior only — not measured observations or query history):

- **Partition/cluster priors**: models with `partition_by` or `cluster_by` scale effective scan by 0.7×
- **Views**: materialized `view` models scale by 0.5×
- **Incrementals**: models with `materialized=incremental` and a `unique_key` scale by `incremental_fraction` (default 5%), preserving lognormal σ
- **Full refresh**: incrementals with `full_refresh=true` skip the incremental discount
- **Unbounded incremental findings** (SQLCOST004/005/019/029): attribution uses full bytes (no incremental discount)

Runs per month resolve from `[cost.models]`, query history, sibling-folder inference, or `default_runs_per_month`.

Project totals sum each model **once** using moment-matched lognormal aggregation (Fenton–Wilkinson).

## Stage 2 — Finding savings attribution

Finding estimates represent **addressable excess cost** (potential monthly savings), not total model spend:

```
savings_fraction = 1 − 1/rule_multiplier
savings ≈ model_monthly_cost × savings_fraction × structure_factor × fan_out_factor
post_fix_cost ≈ model_monthly_cost / ∏(rule_multipliers per model)
potential_savings = model_monthly_cost − post_fix_cost
```

- **Rule multiplier**: lognormal prior per `SQLCOST*` rule (unchanged ranges)
- **Structure factor**: positions within the multiplier range using local SQL features (join count, cross joins, CTE reuse, etc.)
- **Fan-out factor**: bounded boost from downstream model count and exposure dependencies (`1 + log2(1+N) × 0.25`, capped)
- **Per-model cap**: when multiple findings hit one model, savings are scaled so their sum does not exceed ~95% of model monthly cost

Backward-compatible JSON fields:

- `p10_usd_per_month` / `p50_usd_per_month` / `p90_usd_per_month` — **savings** intervals (not total model cost)
- `relative_index` — savings in **GB-months** (dimensionless ranking without pricing)
- New fields: `model_id`, `model_monthly_p50_usd`, `savings_p*_usd_per_month`

## Confidence grades

| Grade | Meaning |
| --- | --- |
| A | Measured from offline query-history export |
| B | Catalog stats or explicit `[cost.sources]` config |
| C | Default size-class prior only |

## Gating

- `fail_on_monthly_delta` — sum of **savings p50** across new (post-baseline) findings
- `fail_on_monthly_delta_gb` — sum of savings `relative_index` (GB-months) when pricing is disabled

Either condition fails the check (OR with severity gate). Failure messages print the computed total and threshold.

## Calibration

`scripts/calibrate_cost_model.py` reads query-history CSV and:

1. Computes 80% interval coverage for bytes-per-run estimates
2. Reports deduplicated model monthly scan volume (TB)
3. Suggests `tb_per_credit_hour` bounds for compute-priced repos

## Implementation

- Rust crate: `costguard-cost` (`model_cost.rs`, `attribution.rs`, `volume.rs`)
- Pipeline: build `ModelCostIndex` → baseline filter → attribute findings → `ProjectCostSummary` on `ScanResult`
- Baseline fingerprints ignore cost fields (`rule_id|path|message`)

## Limitations

- Estimates rank findings and gate PR cost deltas at order-of-magnitude fidelity; not a billing system of record
- Per-model caps prevent double counting but do not model fix interaction effects
- Compute-priced intervals remain wide until query-history calibration narrows `tb_per_credit_hour`
