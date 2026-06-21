# Benchmark evidence

Public snapshot of Costguard precision/recall evidence from real dbt benchmark repos and the corpus regression suite.

Snapshot: [`tests/benchmarks/evidence/v2.5.json`](../../../tests/benchmarks/evidence/v2.5.json).

<!-- generated:evidence:start -->
## Quality ledger

| Evidence | Current value |
| --- | --- |
| Release version | `2.5.0` |
| Evidence snapshot | `tests/benchmarks/evidence/v2.5.json` |
| Full benchmark repositories | 7: jaffle-shop, spellbook, nba-monte-carlo, tuva, mattermost-warehouse, data-infra, ol-data-platform |
| Smoke benchmark repositories | 5: spellbook, nba-monte-carlo, mattermost-warehouse, data-infra, ol-data-platform |
| False-positive registry | clean; 93 verdicts, 30 documented exemptions, 0 open bugs |

## Headline metrics

| Metric | Value | Enterprise gate |
| --- | --- | --- |
| Overall sampled precision | 97.2% | ≥ 80% |
| High-severity sampled precision | 99.8% | ≥ 90% |
| Per-rule sampled precision | see tier table | ≥ 70% each classified rule |
| Rules passing TP census | 44/44 | 44/44 behavioral |

## Precision tiers

- **informational:** 16
- **verified-high:** 27
- **verified-low:** 1
- **verified-medium:** 2

## Example true positives (real dbt repos)

- `SQLCOST001` jaffle-shop `models/marts/order_items.sql:5` — SELECT * in a non-staging model.
- `SQLCOST001` jaffle-shop `models/marts/locations.sql:5` — SELECT * in a non-staging model.
- `SQLCOST002` mattermost-warehouse `transform/snowflake-dbt/models/blp/enterprise_license_mapping.sql:35` — Repeated JSON extraction detected.
- `SQLCOST002` mattermost-warehouse `transform/snowflake-dbt/models/stripe/charges.sql:28` — Repeated JSON extraction detected.
- `SQLCOST003` mattermost-warehouse `transform/snowflake-dbt/models/mattermost/nightly/excludable_servers.sql:40` — Repeated regex operations detected.
- `SQLCOST003` mattermost-warehouse `transform/snowflake-dbt/models/sales/marketing_funnel.sql:74` — Repeated regex operations detected.
- `SQLCOST004` mattermost-warehouse `transform/snowflake-dbt/models/events/hourly/incident_response_events.sql:1` — Incremental model appears to have no unique_key.
- `SQLCOST004` mattermost-warehouse `transform/snowflake-dbt/models/events/hourly/mobile_events.sql:1` — Incremental model appears to have no unique_key.
- `SQLCOST005` mattermost-warehouse `transform/snowflake-dbt/models/orgm/lead_status_hist.sql:1` — Incremental model has no obvious date or partition predicate.
- `SQLCOST005` mattermost-warehouse `transform/snowflake-dbt/models/events/nightly/server_events_by_date.sql:1` — Incremental model has no obvious date or partition predicate.
- `SQLCOST006` spellbook `dbt_subprojects/daily_spellbook/tests/spaceid/bnb/spaceid_registrations_test.sql:13` — Join has no clear equality predicate.
- `SQLCOST006` spellbook `dbt_subprojects/dex/tests/generic/oneinch_no_cross_chain_placeholder_tokens.sql:3` — Join has no clear equality predicate.

Regenerate this page:

```bash
python3 scripts/build_benchmark_evidence.py
```
<!-- generated:evidence:end -->
