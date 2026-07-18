# Cost estimates

Costguard attaches **estimated monthly savings** to each behavioral finding for **prioritization**. Estimates are computed entirely from local files—no warehouse connections or credentials.

> **Disclaimer:** Cost figures are advisory priors derived only from local files (dbt manifest, catalog stats, optional offline exports). They are order-of-magnitude prioritization signals, not a bill and not a guarantee of realized savings. Accuracy depends on input grade (A/B/C); grade C is a pure size prior. Savings assume a fix fully removes the modeled inefficiency and ignore fix-interaction effects. No warehouse connection is made. Severity and confidence are the default enforcement contract; dollar gates are explicit opt-ins for calibrated projects.

Canonical term definitions: [Glossary — Cost estimate terms](../glossary.md#cost-estimate-terms).

## Concepts

### Per-finding fields

| Field | Description |
| --- | --- |
| `relative_index` | **Relative index** — estimated savings in **GB-months** (ranking without pricing) |
| `p10_usd_per_month` / `p50_usd_per_month` / `p90_usd_per_month` | Per-finding **Est. savings** dollar interval when pricing is configured |
| `savings_p10_usd_per_month` / `savings_p50_usd_per_month` / `savings_p90_usd_per_month` | Explicit savings fields (same values as `p*` when priced) |
| `model_id` | dbt model identifier for the finding |
| `model_monthly_p50_usd` | **Model monthly cost** — baseline monthly cost of the underlying model (p50), not the saving |
| `current_cost_p50_usd_per_month` | Model monthly cost on estimated findings (same value as `model_monthly_p50_usd`) |
| `post_fix_cost_p50_usd_per_month` | **Post-fix cost per finding** — that model's modeled monthly cost after fixing this finding |
| `unestimated_reason` | Present when a cost-bearing rule has no multiplier (instead of silent skip) |
| `grade` | Input provenance: **A** (observations or query history), **B** (catalog/config), **C** (size priors) |
| `downstream_model_count` | Transitive downstream dbt models (max 15) for lineage-aware cost context |
| `downstream_monthly_p50_usd` | Sum of downstream models' monthly p50 cost (priced mode; advisory) |

> **v2 semantics:** `p50_usd_per_month` on findings means **estimated savings**, not total model cost. Use `model_monthly_p50_usd` for the model baseline.

### Project cost summary

When `[cost]` is enabled, scan output includes a `cost` block (JSON) or **Cost summary** section (text/markdown):

| Field | Description |
| --- | --- |
| `project_p50_usd` | **Current mapped USD cost** p50 — deduplicated sum of models with pricing or direct monetary observations; `null` when no USD is available |
| `current_cost` | **Current cost** — mapped monthly/annual USD with uncertainty plus full-project `gb_months_p50` (`CostFigure`) |
| `post_fix_cost` | **Post-fix cost** — counterfactual cost if all current findings were fixed |
| `potential_savings` | **Potential savings** — `current_cost − post_fix_cost` (top-down, per model) |
| `coverage` | **Coverage** — mapped-spend fraction, separate USD model count/fraction, observation age, rules estimated/unestimated |
| `pr_impact` | **PR impact** — base vs head delta in PR mode (`introduced`, `avoided`, `net`, `efficiency`, `volume`, `blast_radius`); `net.monthly_p50` gates `fail_on_pr_cost_increase` |
| `realized_savings` | **Realized savings** — before/after observation bundles (`observations_before` + `observations_after`) |
| `project_gb_months` | Sum of model scan volumes in GB-months |
| `savings_p50_usd` | **Addressable finding savings (deduplicated)** — bottom-up sum of per-finding savings; gates `fail_on_monthly_delta` |
| `top_models` | Top 5 models by USD cost at 100% USD coverage, otherwise by full-project GB-month volume |
| `grade_a` / `grade_b` / `grade_c` | Count of models by input grade |

### Two savings numbers

Cost summary reports two related but differently computed savings totals:

| Label (text/markdown) | Field | How computed | Use |
| --- | --- | --- | --- |
| **Potential savings (current − post-fix)** | `potential_savings` | Top-down: `current_cost − post_fix_cost`, per model then aggregated | Whole-project counterfactual reduction |
| **Addressable savings on flagged findings (deduplicated)** | `savings_p50_usd` | Bottom-up: sum of per-finding attributed shares with structure/fan-out weights and per-model caps (~95% max) | PR/scan cost gating (`--fail-on-cost-delta`) |

They are close but not identical by construction. `--fail-on-cost-delta` uses the addressable finding sum. The separate project-wide `fail_on_pr_cost_increase` gate uses PR net cost, not either savings total.

## Modes

1. **Relative index only** — enable `[cost]` without `[cost.pricing]`. Findings ranked by GB-month savings; gate with `fail_on_monthly_delta_gb`.
2. **Scan-priced dollars** — BigQuery on-demand, Athena, Trino-on-S3: set `[cost.pricing] model = "scan"` and `usd_per_tb`.
3. **Compute-priced dollars** — Snowflake credits, Databricks DBUs: set `model = "compute"`, `usd_per_credit`, and `tb_per_credit_hour` range.

Byte volume and USD are independent tracks. Unpriced bytes never appear in a USD field. Direct monetary observations can provide mapped USD without global scan pricing; if only part of the project has monetary coverage, reports label those totals as mapped USD and still show full-project GB-month volume. `CostFigure.monthly_p*` and `annual_p50` are always USD, while `gb_months_p50` is always monthly scan volume in GB.

## Benchmark repo cost configs

External benchmark repos in [`tests/benchmarks/repos.toml`](../../../tests/benchmarks/repos.toml) can carry committed Grade C cost priors under [`tests/benchmarks/cost-configs/`](../../../tests/benchmarks/cost-configs/). Before each cost-enabled scan, the benchmark harness copies `{repo}.toml` into the cached checkout as `costguard.toml`.

**These configs are estimates only.** Dollar figures use size priors (Grade C), not measured warehouse spend. Each file includes a header disclaimer; treat outputs as illustrative prioritization signals until query history (Grade A) or catalog stats (Grade B) are available.

## Example configuration

```toml
[cost]
enabled = true
interval = 0.80
default_runs_per_month = 30
default_table_size = "medium"
fail_on_monthly_delta = 500
fail_on_monthly_delta_gb = 1000

[gate]
block_only_new = true
fail_on_pr_cost_increase = 1000

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
costguard pr --fail-on-pr-cost-increase 1000
costguard scan --cost --min-cost-coverage 0.8
costguard explain models/marts/fct.sql --cost
costguard cost report . --manifest target/manifest.json
```

- `--cost` on `scan`, `pr`, and `explain` enables cost annotation
- `--fail-on-cost-delta` gates on **addressable finding savings** p50 (deduplicated sum; also enables cost)
- `--fail-on-pr-cost-increase` gates on project-wide priced **net PR impact** p50; it requires `[cost.pricing].model = "scan"` or `"compute"` and also enables cost
- With `block_only_new`, addressable-savings gates count introduced/regressed diagnostics only. The net PR gate already compares the full base and head project cost.
- `--min-cost-coverage` gates on **mapped-spend coverage** (`coverage.mapped_spend_fraction`; also enables cost). Config equivalent: `[cost].min_mapped_spend_fraction`
- `costguard cost report` renders a local cost prioritization summary without requiring findings
- `costguard cost normalize` converts offline warehouse exports into the normalized metadata-only cost schema

## Output

Text and markdown scans append:

- Per-finding savings line (`Est. savings: …`, with model cost when priced)
- Top findings by estimated monthly savings
- Cost summary with current/post-fix/potential savings, addressable finding savings, grade mix, top models, and an advisory disclaimer footer

JSON schema version is **4** with an optional top-level `cost` object. Additive fields `CostFigure.gb_months_p50`, `coverage.models_with_usd`, and `coverage.usd_coverage_fraction` distinguish full volume from mapped USD. Monetary fields are `null` when unavailable; `relative_index` remains the per-finding GB-month measure. Per-finding estimates include additive `prior_basis`: `config-override`, `rule-prior:generic`, or `warehouse-prior:<warehouse>`; explicit `[cost.rules.RULE_ID]` multipliers always win.

## GitHub Action

```yaml
- uses: hypertrial/costguard/.github/actions/costguard@v2.7.0
  with:
    cost: "true"
    fail-on-cost-delta: "500"
    block-only-new: "true"
    fail-on-pr-cost-increase: "1000"
```

## Calibration

Offline query-history enrichment is shipped through `[cost.inputs].query_history` and `costguard cost normalize`. Tune `[cost.pricing]` using those exports, then validate bytes-per-run coverage and compute conversion bounds against your repo's observed spend before enabling dollar gates.

For local dlt/dbt/DuckDB/Dagster-style pipelines, export one row per dbt model and time window, then normalize it:

```bash
costguard cost normalize pipeline_observations.csv costguard-observations.json \
  --source pipeline \
  --organization acme \
  --repository acme/warehouse \
  --provenance dagster-run-2026-07-05 \
  --model-mapping model-mapping.json
```

Minimum columns are a model identifier such as `model_id`, `relation`, or `asset_key`, plus `window_start` and `window_end`. Optional columns include `executions`/`run_count`, `duration_seconds`/`duration_ms`, `bytes_processed`, `credits`, and `cost_usd`; row counts are accepted but ignored by v2.7.0. Point `[cost.inputs].observations` at the generated JSON after reviewing pricing calibration.

## Related

- [Configuration](configuration.md) — full `[cost]` schema
- [Cost model design](../../design/cost-model.md) — two-stage math
- [Privacy](../getting-started/privacy.md) — local-only inputs
