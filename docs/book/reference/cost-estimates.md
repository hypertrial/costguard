# Cost estimates

Costguard attaches **estimated monthly savings** to each behavioral finding for **prioritization**. Estimates are advisory—not a billing system of record. Severity and confidence remain the enforcement contract. Estimates are computed entirely from local files—no warehouse connections or credentials.

## Concepts

### Per-finding fields

| Field | Description |
| --- | --- |
| `relative_index` | Estimated savings in **GB-months** (ranking without pricing) |
| `p10_usd_per_month` / `p50_usd_per_month` / `p90_usd_per_month` | **Savings** dollar interval when pricing is configured |
| `savings_p10_usd_per_month` / `savings_p50_usd_per_month` / `savings_p90_usd_per_month` | Explicit savings fields (same values as `p*` when priced) |
| `model_id` | dbt model identifier for the finding |
| `model_monthly_p50_usd` | Resolved monthly cost of the underlying model (p50) |
| `current_cost_p50_usd_per_month` | Same as `model_monthly_p50_usd` on estimated findings |
| `post_fix_cost_p50_usd_per_month` | Counterfactual monthly cost after fixing the finding |
| `unestimated_reason` | Present when a cost-bearing rule has no multiplier (instead of silent skip) |
| `grade` | Input provenance: **A** (observations or query history), **B** (catalog/config), **C** (size priors) |
| `basis` | Human-readable derivation string |

> **v2 semantics:** `p50_usd_per_month` on findings now means **estimated savings**, not total model cost. Use `model_monthly_p50_usd` for the model baseline.

### Project cost summary

When `[cost]` is enabled, scan output includes a `cost` block (JSON) or **Cost summary** section (text/markdown):

| Field | Description |
| --- | --- |
| `project_p50_usd` | Deduplicated sum of per-model monthly costs (priced mode) |
| `current_cost` | Project monthly/annual cost with uncertainty (`CostFigure`) |
| `post_fix_cost` | Counterfactual cost if all current findings were fixed |
| `potential_savings` | `current_cost − post_fix_cost` |
| `coverage` | Mapped-spend fraction, observation age, rules estimated/unestimated |
| `pr_impact` | Base vs head delta in PR mode (`introduced`, `avoided`, `net`, `efficiency`, `volume`) |
| `realized_savings` | Before/after observation bundles (`observations_before` + `observations_after`) |
| `project_gb_months` | Sum of model scan volumes in GB-months |
| `savings_p50_usd` | Deduplicated sum of new finding savings |
| `top_models` | Top 5 models by monthly cost |
| `grade_a` / `grade_b` / `grade_c` | Count of models by input grade |

## Modes

1. **Relative index only** — enable `[cost]` without `[cost.pricing]`. Findings ranked by GB-month savings; gate with `fail_on_monthly_delta_gb`.
2. **Scan-priced dollars** — BigQuery on-demand, Athena, Trino-on-S3: set `[cost.pricing] model = "scan"` and `usd_per_tb`.
3. **Compute-priced dollars** — Snowflake credits, Databricks DBUs: set `model = "compute"`, `usd_per_credit`, and `tb_per_credit_hour` range.

## Example configuration

```toml
[cost]
enabled = true
interval = 0.80
default_runs_per_month = 30
default_table_size = "medium"
fail_on_monthly_delta = 500
fail_on_monthly_delta_gb = 1000

[cost.pricing]
model = "scan"
usd_per_tb = 6.25

[cost.inputs]
catalog = "target/catalog.json"
observations = "cost/observations.json"
observations_before = "cost/before.json"
observations_after = "cost/after.json"
query_history = "exports/jobs_30d.csv"

[cost.sources."raw.events"]
bytes = "12TB"

[cost.models."marts.fct_orders"]
runs_per_month = 720

[cost.rules.SQLCOST014]
multiplier = { p10 = 2, p90 = 6 }
```

## CLI

```bash
costguard scan --cost
costguard scan --cost --fail-on-cost-delta 500
costguard pr --cost --fail-on-cost-delta 500
costguard explain models/marts/fct.sql --cost
costguard cost report . --manifest target/manifest.json
```

- `--cost` on `scan`, `pr`, and `explain` enables cost annotation
- `--fail-on-cost-delta` gates on deduplicated **savings p50** (also enables cost)
- `costguard cost report` renders a local cost prioritization summary without requiring findings
- `costguard cost normalize` converts offline warehouse exports into the normalized metadata-only cost schema

## Output

Text and markdown scans append:

- Per-finding savings line (`Est. savings: …`)
- Top findings by estimated monthly savings
- Cost summary with project total, grade mix, and top models

JSON schema version is **3** with an optional top-level `cost` object.

## GitHub Action

```yaml
- uses: hypertrial/costguard/.github/actions/costguard@v2.1.0
  with:
    cost: "true"
    fail-on-cost-delta: "500"
```

## Calibration

```bash
python3 scripts/calibrate_cost_model.py exports/jobs_30d.csv
python3 scripts/calibrate_cost_model.py exports/jobs_30d.csv --json
```

Reports bytes-per-run interval coverage, model monthly scan volume, and suggested compute conversion bounds.

## Related

- [Configuration](configuration.md) — full `[cost]` schema
- [Cost model design](../../design/cost-model.md) — two-stage math
- [Privacy](../getting-started/privacy.md) — local-only inputs
