# Benchmark evidence

Public snapshot of Costguard precision/recall evidence from real dbt benchmark repos and the corpus regression suite.

Snapshot: [`tests/benchmarks/evidence/v2.6.json`](../../../tests/benchmarks/evidence/v2.6.json).

<!-- generated:evidence:start -->
## Quality ledger

| Evidence | Current value |
| --- | --- |
| Release version | `2.6.0` |
| Evidence snapshot | `tests/benchmarks/evidence/v2.6.json` |
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

- **informational:** 17
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

## Reproduce the proof

The evidence chain is intentionally split so one aggregate metric cannot hide a regression:

| Proof | Source | Validation |
| --- | --- | --- |
| Real-repo precision and exemptions | [`tests/benchmarks/fp_registry.toml`](../../../tests/benchmarks/fp_registry.toml) plus vendored repository pins | `python3 scripts/benchmark_external_repo.py --all-vendored` and `python3 scripts/build_benchmark_evidence.py --check` |
| Behavioral-rule true-positive coverage | [`tests/benchmarks/rule_tp_evidence.json`](../../../tests/benchmarks/rule_tp_evidence.json) | `python3 scripts/rule_tp_census.py --json` (use `--emit-evidence` only for an intentional refresh) |
| Base/head regression semantics | [`crates/costguard-core/tests/pr_replay.rs`](../../../crates/costguard-core/tests/pr_replay.rs) | `cargo test -p costguard-core --test pr_replay --locked` |
| Net/introduced/avoided PR cost | [`crates/costguard-core/tests/pr_cost_impact.rs`](../../../crates/costguard-core/tests/pr_cost_impact.rs) | `cargo test -p costguard-core --test pr_cost_impact --locked` |
| Output, Action, and receipt contract | Output/CLI unit suites and [`scripts/tests/test_action_contract.py`](../../../scripts/tests/test_action_contract.py) | `cargo test -p costguard-output --locked`, `cargo test -p costguard-cli --test cli --locked`, and `python3 -m unittest discover -s scripts/tests -p 'test_*.py'` |
| Release qualification | Exact-SHA [`ci.yml`](../../../.github/workflows/ci.yml) `pr-gate` push evidence plus an independently dispatched [`benchmark.yml`](../../../.github/workflows/benchmark.yml) `full-evidence-gate` | The push gate includes synthetic scale; the benchmark gate re-runs the full local qualification and required external support matrix before the signed tag release proceeds |
| 10k base replay | Version-4 [`scale_check.py`](../../../scripts/scale_check.py) report | One warm-up and two measured `costguard pr --base HEAD~1` replays must prove a committed delta and processed base context |

Documentation and committed evidence are part of the proof surface: run `python3 scripts/check_docs.py` and `mdbook build` after changing rules, metrics, outputs, or public claims.
