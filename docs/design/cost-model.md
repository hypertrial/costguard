# Cost model design (v2)

Costguard estimates monthly spend using a **two-stage, model-centric** lognormal product model. All inputs are local files; the scanner never connects to a warehouse.

Canonical term definitions: [Glossary — Cost estimate terms](../book/glossary.md#cost-estimate-terms).

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
4. `catalog.json` `stats.row_count × avg_row_bytes` (default 200) — grade **B**
5. `[cost.sources]` bytes/rows — grade **B**
6. `default_table_size` prior — grade **C**

Adjustments before pricing (catalog, config source, and size prior only — not measured observations or query history):

Adjustments before pricing (catalog, config source, and size prior only — not measured observations or query history). Priors are warehouse-specific:

| Warehouse | Partition | Cluster | Notes |
| --- | --- | --- | --- |
| BigQuery | 0.5× | 0.7× | Strong partition pruning on `partition_by` |
| Snowflake | 0.75× | 0.8× | Clustering depth not modeled; `cluster_by` only |
| Databricks | 0.75× | 0.75× | File-skipping prior (ponytail: no shuffle model) |
| Generic / other | 0.7× | 0.7× | Combined partition or cluster presence |

- **Views**: materialized `view` models scale by 0.5× (all warehouses)
- **Incrementals**: models with `materialized=incremental` and a `unique_key` scale by `incremental_fraction` (default 5%), preserving lognormal σ
- **Full refresh**: incrementals with `full_refresh=true` skip the incremental discount
- **Unbounded incremental findings** (SQLCOST004/005/019/029): attribution uses full bytes (no incremental discount)

Runs per month resolve from `[cost.models]`, query history, sibling-folder inference, or `default_runs_per_month`.

**Current cost** (`current_cost`) sums each model **once** using moment-matched lognormal aggregation (Fenton–Wilkinson).

## Stage 2 — Finding savings attribution

Finding estimates represent **addressable excess cost** (per-finding Est. savings), not total model spend:

```
savings_fraction = 1 − 1/rule_multiplier
per_finding_savings ≈ model_monthly_cost × savings_fraction × structure_factor × fan_out_factor
post_fix_cost ≈ model_monthly_cost / ∏(rule_multipliers per model)
potential_savings = current_cost − post_fix_cost   (per model, then aggregated)
```

- **Rule multiplier**: lognormal prior per `SQLCOST*` rule (unchanged ranges)
- **Structure factor**: positions within the multiplier range using local SQL features (join count, cross joins, CTE reuse, etc.)
- **Fan-out factor**: bounded boost from downstream model count and exposure dependencies (`1 + log2(1+N) × 0.25`, capped)
- **Per-model cap**: when multiple findings hit one model, savings are scaled so their sum does not exceed ~95% of model monthly cost

### Two savings totals

| Term | Computation | Role |
| --- | --- | --- |
| **Potential savings** (`potential_savings`) | Top-down: `current_cost − post_fix_cost`, per model | Whole-project counterfactual |
| **Addressable finding savings** (`savings_p50_usd`) | Bottom-up: sum of capped per-finding shares | PR/scan cost gating |

They are close but not identical; gating uses the addressable finding sum.

Backward-compatible JSON fields:

- `p10_usd_per_month` / `p50_usd_per_month` / `p90_usd_per_month` — **savings** intervals (not total model cost)
- `relative_index` — savings in **GB-months** for volume-based ranking without pricing
- New fields: `model_id`, `model_monthly_p50_usd`, `savings_p*_usd_per_month`

## Confidence grades

| Grade | Meaning |
| --- | --- |
| A | Measured from observations or offline query-history export |
| B | Catalog stats or explicit `[cost.sources]` config |
| C | Default size-class prior only |

## Gating

- `fail_on_monthly_delta` — **addressable finding savings** p50 across post-baseline findings; with `block_only_new`, only introduced/regressed diagnostics contribute
- `fail_on_monthly_delta_gb` — sum of savings `relative_index` (GB-months); with `block_only_new`, only introduced/regressed diagnostics contribute
- `fail_on_pr_cost_increase` — project-wide `pr_impact.net.monthly_p50` in priced PR mode; equal to or above the threshold fails, negative/avoided net cost passes

Either condition fails the check (OR with severity gate). Failure messages print the computed total and threshold.

## Calibration

Offline query-history input is shipped through `[cost.inputs].query_history`, and `costguard cost normalize` converts supported warehouse exports to the normalized observation schema. Tune `[cost.pricing]` with local exports and validate interval coverage manually.

## Implementation

- Rust crate: `costguard-cost` (`model_cost.rs`, `attribution.rs`, `volume.rs`)
- Pipeline: build `ModelCostIndex` → baseline filter → attribute findings → `ProjectCostSummary` on `ScanResult`
- Baseline fingerprints ignore cost fields (`rule_id|path|message`)

## Limitations and disclaimer

Cost figures are advisory priors derived only from local files. They are order-of-magnitude prioritization signals, not a bill and not a guarantee of realized savings.

- Accuracy depends on input grade (A/B/C); grade C is a pure size prior with no measured spend
- Savings assume a fix fully removes the modeled inefficiency; fix-interaction effects are not modeled
- Per-model caps prevent double counting but do not model overlapping fix benefits
- Compute-priced intervals remain wide until query-history calibration narrows `tb_per_credit_hour`
- No warehouse connection is made; severity and confidence are the default gate, while calibrated cost gates are explicit opt-ins
