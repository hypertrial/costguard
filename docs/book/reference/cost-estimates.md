# Cost estimates

Costguard can attach **estimated monthly cost intervals** to each behavioral finding. Estimates are computed entirely from local files—no warehouse connections or credentials.

## Concepts

Each finding with a cost estimate includes:

| Field | Description |
| --- | --- |
| `relative_index` | Dimensionless impact score (GB-equivalent × rule multiplier × runs) |
| `p10_usd_per_month` / `p50_usd_per_month` / `p90_usd_per_month` | Dollar interval when pricing is configured |
| `grade` | Input provenance: **A** (query history), **B** (catalog/config), **C** (size priors) |
| `basis` | Human-readable derivation string |

Intervals default to an **80% band** (`p10`–`p90`). Values are rounded to two significant figures in text output.

## Modes

1. **Relative index only** — enable `[cost]` without `[cost.pricing]`. Findings are ranked by `relative_index`.
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

[cost.pricing]
model = "scan"
usd_per_tb = 6.25

[cost.inputs]
catalog = "target/catalog.json"
query_history = "exports/jobs_30d.csv"

[cost.sources."raw.events"]
bytes = "12TB"

[cost.models."marts.fct_orders"]
runs_per_month = 720

[cost.rules.SQLCOST014]
multiplier = { p10 = 2, p90 = 6 }
```

## Input sources (priority order)

1. **Query-history export (grade A)** — CSV with columns `model_or_table`, `bytes_per_run`, optional `runs_per_month`.
2. **`target/catalog.json` (grade B)** — from `dbt docs generate` on BigQuery/Snowflake adapters (`stats.num_bytes`).
3. **`[cost.sources]` overrides (grade B)** — explicit bytes or row counts in `costguard.toml`.
4. **Size-class priors (grade C)** — `default_table_size` (`small`, `medium`, `large`, `xlarge`).

## CLI flags

```bash
costguard scan --cost
costguard pr --cost --fail-on-cost-delta 500
```

When `--cost` is passed without a `[cost]` section, Costguard uses default priors.

## GitHub Action

```yaml
- uses: hypertrial/costguard/.github/actions/costguard@v1
  with:
    cost: "true"
    fail-on-cost-delta: "500"
```

Cost delta gating sums **p50** across post-baseline (new) findings only.

## Exporting query history

### BigQuery

```sql
SELECT
  REGEXP_EXTRACT(query, r'\\`([^.]+\\.[^.]+\\.[^`]+)\\`') AS model_or_table,
  AVG(total_bytes_billed) AS bytes_per_run,
  COUNT(*) * 30.0 / DATE_DIFF(MAX(DATE(creation_time)), MIN(DATE(creation_time)), DAY) AS runs_per_month
FROM `region-us`.INFORMATION_SCHEMA.JOBS
WHERE creation_time >= TIMESTAMP_SUB(CURRENT_TIMESTAMP(), INTERVAL 30 DAY)
  AND statement_type = 'SELECT'
GROUP BY 1;
```

Export the result to CSV and point `[cost.inputs].query_history` at the file.

### Snowflake

```sql
SELECT
  REGEXP_SUBSTR(query_text, '([A-Z0-9_]+\\.[A-Z0-9_]+\\.[A-Z0-9_]+)', 1, 1, 'e', 1) AS model_or_table,
  AVG(bytes_scanned) AS bytes_per_run,
  COUNT(*) * 30.0 / NULLIF(DATEDIFF('day', MIN(start_time), MAX(start_time)), 0) AS runs_per_month
FROM snowflake.account_usage.query_history
WHERE start_time >= DATEADD('day', -30, CURRENT_TIMESTAMP())
GROUP BY 1;
```

## Calibration

Fit compute conversion factors and validate interval coverage with:

```bash
python3 scripts/calibrate_cost_model.py exports/jobs_30d.csv
```

The script reports what fraction of 80% intervals contain actual bytes-billed values (target band 60–95%) and suggests `tb_per_credit_hour` bounds.

## Related

- [Configuration](configuration.md) — full `[cost]` schema
- [Cost model design](../../design/cost-model.md) — lognormal math
- [Privacy](../getting-started/privacy.md) — local-only inputs
