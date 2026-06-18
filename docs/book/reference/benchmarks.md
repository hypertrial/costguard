# Benchmark evidence

Public snapshot of Costguard precision/recall evidence from real dbt benchmark repos (Spellbook, jaffle-shop, mattermost-warehouse, data-infra) and the corpus regression suite.

Snapshot: [`tests/benchmarks/evidence/v2.4.json`](../../../tests/benchmarks/evidence/v2.4.json) (generated 2026-06-18).

<!-- generated:evidence:start -->
## Headline metrics

| Metric | Value | Enterprise gate |
| --- | --- | --- |
| Overall sampled precision | 97.1% | ≥ 80% |
| High-severity sampled precision | 100.0% | ≥ 90% |
| Per-rule sampled precision | see tier table | ≥ 70% each classified rule |
| Rules passing TP census | 44/45 | 44/44 behavioral |

## Precision tiers

- **informational:** 16
- **verified-high:** 27
- **verified-low:** 2

## Example true positives (real dbt repos)

- `SQLCOST001` data-infra `warehouse/models/intermediate/transit_database/dimensions/int_transit_database__services_dim.sql:49` — SELECT * in a non-staging model.
- `SQLCOST001` data-infra `warehouse/models/intermediate/transit_database/dimensions/int_transit_database__modes_dim.sql:27` — SELECT * in a non-staging model.
- `SQLCOST002` spellbook `dbt_subprojects/dex/models/_projects/uniswap/optimism/uniswap_optimism_ovm1_pool_mapping.sql:749` — Repeated JSON extraction detected.
- `SQLCOST002` spellbook `dbt_subprojects/daily_spellbook/models/_projects/eigenlayer/ethereum/eigenlayer_ethereum_slashing_withdrawal_queued_flattened.sql:33` — Repeated JSON extraction detected.
- `SQLCOST003` spellbook `dbt_subprojects/daily_spellbook/models/op/retropgf/optimism/op_retropgf_optimism_round2_voters.sql:12` — Repeated regex operations detected.
- `SQLCOST003` data-infra `warehouse/macros/littlepay_staging_transforms.sql:15` — Repeated regex operations detected.
- `SQLCOST004` mattermost-warehouse `transform/snowflake-dbt/models/events/hourly/incident_response_events.sql:1` — Incremental model appears to have no unique_key.
- `SQLCOST004` mattermost-warehouse `transform/snowflake-dbt/models/events/hourly/mobile_events.sql:1` — Incremental model appears to have no unique_key.
- `SQLCOST005` mattermost-warehouse `transform/snowflake-dbt/models/orgm/lead_status_hist.sql:1` — Incremental model has no obvious date or partition predicate.
- `SQLCOST005` data-infra `warehouse/models/staging/audit/stg_audit__cloudaudit_googleapis_com_data_access.sql:1` — Incremental model has no obvious date or partition predicate.
- `SQLCOST006` mattermost-warehouse `transform/snowflake-dbt/models/orgm/opportunityfieldhistory.sql:12` — Join has no clear equality predicate.
- `SQLCOST006` spellbook `dbt_subprojects/dex/models/_projects/oneinch/_meta/oneinch_blockchains.sql:20` — Join has no clear equality predicate.

Regenerate this page:

```bash
python3 scripts/build_benchmark_evidence.py
```
<!-- generated:evidence:end -->
