# Rule TP coverage (review census repos)

Generated: 2026-06-18
Review census corpus: spellbook (Trino), jaffle-shop (duckdb), mattermost-warehouse (Snowflake), data-infra (BigQuery). These census repos set `compile_dbt = true`; Snowflake/BigQuery use `compile_best_effort = true` when offline dummy profiles cannot reach the warehouse. The broader support matrix also includes required NBA Monte Carlo and Tuva external benchmarks.  
Pass bar: **≤100 cost-ranked examples per rule** (100% if fewer) with **0 `fp_bug` + 0 `unknown`** in the examined sample.  
Tool: [`scripts/rule_tp_census.py`](../../scripts/rule_tp_census.py); evidence: [`tests/benchmarks/rule_tp_evidence.json`](../../tests/benchmarks/rule_tp_evidence.json). Supplemental tail evidence lives in [`tests/benchmarks/evidence/rule_tp_stratified_supplemental.json`](../../tests/benchmarks/evidence/rule_tp_stratified_supplemental.json).

The census covers the 44 cost/behavioral rules (`SQLCOST001`–`SQLCOST044`). `SQLCOST045` (stale dbt manifest) is excluded like infrastructure rules `SQLCOST023`–`027` and is validated by [`crates/costguard-core/tests/stale_manifest.rs`](../../crates/costguard-core/tests/stale_manifest.rs).

For the full manual review workflow (adjudication loop, registry contracts, bucket taxonomy), see [Manual rule review playbook](manual-rule-review.md).

## Verification methodology

Each rule was validated by:

1. Scanning all four benchmark repos at pinned commits ([`tests/benchmarks/repos.toml`](../../tests/benchmarks/repos.toml)) with compiled manifests where available and cost enabled for ranking.
2. Classifying every diagnostic into **TP**, **exempt**, **fp_bug**, or **unknown** using [`scripts/bucket_rule_diagnostics.py`](../../scripts/bucket_rule_diagnostics.py) buckets, [`tests/benchmarks/fp_registry.toml`](../../tests/benchmarks/fp_registry.toml) verdicts, and optional `class` on `verdict = "fp"` rows.
3. Cost-ranking findings by `savings_p50_usd_per_month` and examining up to 100 per rule (100% when total &lt;100).
4. Applying the pass bar: `fp_bug == 0` and `unknown == 0` in the examined sample; infrastructure rules (`SQLCOST023`–`027`) excluded.
5. Sampling high-volume tails by `(repo, rule, bucket)` after the primary cost-ranked sample for rules with large unreviewed tails.

Rules with **zero findings** pass as `vacuous_clean` when corpus `expect_rules` / `forbid_rules` and registry buckets prove the rule is wired correctly.

## How to reproduce

```bash
# Requires cached checkouts (~/.cache/costguard/benchmarks/) and release costguard binary
python3 scripts/rule_tp_census.py \
  --repos spellbook jaffle-shop mattermost-warehouse data-infra \
  --emit-evidence

# Force dbt recompile if manifest stale
python3 scripts/rule_tp_census.py --force-compile --emit-evidence

# Iterate on one rule
python3 scripts/rule_tp_census.py --rule SQLCOST006 --sample-cap 100

# Supplemental high-volume tail evidence
python3 scripts/rule_tp_census.py --emit-stratified-evidence
```

Exit code `0` means 44/44 PASS. Refresh this doc's scoreboard after intentional rule or registry changes.

## Summary
- **44/44 rules PASS**

| Rule | Total | Examined | TP | Exempt | FP bug | Unknown | Pass reason |
|------|-------|----------|----|--------|--------|---------|-------------|
| SQLCOST001 | 441 | 100 | 100 | 0 | 0 | 0 | sampled_100_of_441 |
| SQLCOST002 | 396 | 100 | 100 | 0 | 0 | 0 | sampled_100_of_396 |
| SQLCOST003 | 32 | 32 | 32 | 0 | 0 | 0 | fully_examined |
| SQLCOST004 | 91 | 91 | 91 | 0 | 0 | 0 | fully_examined |
| SQLCOST005 | 12 | 12 | 11 | 1 | 0 | 0 | fully_examined |
| SQLCOST006 | 38 | 38 | 33 | 5 | 0 | 0 | fully_examined |
| SQLCOST007 | 57 | 57 | 57 | 0 | 0 | 0 | fully_examined |
| SQLCOST008 | 194 | 100 | 100 | 0 | 0 | 0 | sampled_100_of_194 |
| SQLCOST009 | 0 | 0 | 0 | 0 | 0 | 0 | vacuous_clean |
| SQLCOST010 | 0 | 0 | 0 | 0 | 0 | 0 | vacuous_clean |
| SQLCOST011 | 0 | 0 | 0 | 0 | 0 | 0 | vacuous_clean |
| SQLCOST012 | 66 | 66 | 49 | 17 | 0 | 0 | fully_examined |
| SQLCOST013 | 187 | 100 | 100 | 0 | 0 | 0 | sampled_100_of_187 |
| SQLCOST014 | 938 | 100 | 93 | 7 | 0 | 0 | sampled_100_of_938 |
| SQLCOST015 | 346 | 100 | 100 | 0 | 0 | 0 | sampled_100_of_346 |
| SQLCOST016 | 0 | 0 | 0 | 0 | 0 | 0 | vacuous_clean |
| SQLCOST017 | 34 | 34 | 11 | 23 | 0 | 0 | fully_examined |
| SQLCOST018 | 339 | 100 | 100 | 0 | 0 | 0 | sampled_100_of_339 |
| SQLCOST019 | 123 | 100 | 100 | 0 | 0 | 0 | sampled_100_of_123 |
| SQLCOST020 | 22 | 22 | 22 | 0 | 0 | 0 | fully_examined |
| SQLCOST021 | 0 | 0 | 0 | 0 | 0 | 0 | vacuous_clean |
| SQLCOST022 | 0 | 0 | 0 | 0 | 0 | 0 | vacuous_clean |
| SQLCOST023 | 0 | 0 | 0 | 0 | 0 | 0 | infrastructure_na |
| SQLCOST024 | 0 | 0 | 0 | 0 | 0 | 0 | infrastructure_na |
| SQLCOST025 | 0 | 0 | 0 | 0 | 0 | 0 | infrastructure_na |
| SQLCOST026 | 1 | 1 | 0 | 0 | 0 | 1 | infrastructure_na |
| SQLCOST027 | 7650 | 100 | 0 | 0 | 0 | 100 | infrastructure_na |
| SQLCOST028 | 171 | 100 | 100 | 0 | 0 | 0 | sampled_100_of_171 |
| SQLCOST029 | 114 | 100 | 100 | 0 | 0 | 0 | sampled_100_of_114 |
| SQLCOST030 | 2 | 2 | 2 | 0 | 0 | 0 | fully_examined |
| SQLCOST031 | 12 | 12 | 12 | 0 | 0 | 0 | fully_examined |
| SQLCOST032 | 0 | 0 | 0 | 0 | 0 | 0 | vacuous_clean |
| SQLCOST033 | 11 | 11 | 11 | 0 | 0 | 0 | fully_examined |
| SQLCOST034 | 0 | 0 | 0 | 0 | 0 | 0 | vacuous_clean |
| SQLCOST035 | 124 | 100 | 100 | 0 | 0 | 0 | sampled_100_of_124 |
| SQLCOST036 | 61 | 61 | 61 | 0 | 0 | 0 | fully_examined |
| SQLCOST037 | 46 | 46 | 46 | 0 | 0 | 0 | fully_examined |
| SQLCOST038 | 447 | 100 | 100 | 0 | 0 | 0 | sampled_100_of_447 |
| SQLCOST039 | 74 | 74 | 74 | 0 | 0 | 0 | fully_examined |
| SQLCOST040 | 12 | 12 | 12 | 0 | 0 | 0 | fully_examined |
| SQLCOST041 | 7 | 7 | 7 | 0 | 0 | 0 | fully_examined |
| SQLCOST042 | 604 | 100 | 100 | 0 | 0 | 0 | sampled_100_of_604 |
| SQLCOST043 | 0 | 0 | 0 | 0 | 0 | 0 | vacuous_clean |
| SQLCOST044 | 6 | 6 | 6 | 0 | 0 | 0 | fully_examined |

## Platform-specific notes
- **SQLCOST021** (BigQuery wildcard): no findings in data-infra scan; recall corpus proves rule fires.
- **SQLCOST042** (BigQuery partition filter): 604 TP examples on data-infra (100 examined in sample).
- **SQLCOST043** (Snowflake merge pruning): no findings on mattermost-warehouse; recall corpus covers.
- **SQLCOST028** (partition/cluster config): 171 TP on data-infra BigQuery marts.
- **SQLCOST027** (parse markers): high-volume infrastructure-only markers are now grouped in supplemental evidence by source pattern; top causes remain Jinja templates and dbt config wrappers.

## Vacuous and infrastructure rules

| Pass reason | Meaning | Examples |
| --- | --- | --- |
| `vacuous_clean` | No findings in four-repo scan; corpus + registry prove rule behavior | SQLCOST009–011, SQLCOST021–022, SQLCOST032, SQLCOST034, SQLCOST043 |
| `infrastructure_na` | SQLCOST023–027 — scan/parse/metadata markers, not cost anti-patterns | SQLCOST027 parse-failure markers counted but not behaviorally adjudicated |
| `fully_examined` | Fewer than 100 total findings; 100% of findings examined | SQLCOST006, SQLCOST017, SQLCOST030–031, SQLCOST033, SQLCOST040, SQLCOST044 |
| `sampled_100_of_N` | Cost-ranked top 100 of N total findings examined | SQLCOST001, SQLCOST038, SQLCOST042 |

## Documented exemptions in sample

Rules may PASS with `exempt > 0` when registry rows use `class = "exempt"` for repo-specific-valid patterns:

| Rule | Exempt (in sample) | Typical buckets |
| --- | --- | --- |
| SQLCOST005 | 1 | `date_trunc_present` |
| SQLCOST006 | 5 | `equality_join` (residual regex-only / Jinja-heavy joins after equality fallback improvements) |
| SQLCOST012 | 17 | `cross_join_unnest`, `date_spine_cross_join`, `group_by_comma_fp`, … |
| SQLCOST014 | 7 | `other` (single-use CTE homonyms) |
| SQLCOST017 | 23 | `date_trunc_join`, `symmetric_normalize`, `lower_trim`, `coalesce_key` |

Paydown workflow: [Manual rule review playbook — Residual FP policy](manual-rule-review.md#residual-fp-policy).

## Supplemental tail review

Generated: 2026-06-18
Supplemental evidence samples findings not already in the primary cost-ranked sample, stratified by `(repo, rule, bucket)`. Behavioral supplemental samples have **0 `fp_bug`** and **0 `unknown`**. Evidence lives in [`tests/benchmarks/evidence/rule_tp_stratified_supplemental.json`](../../tests/benchmarks/evidence/rule_tp_stratified_supplemental.json).

| Rule | Tail total | Supplemental examined | TP | Exempt | FP bug | Unknown |
|------|------------|-----------------------|----|--------|--------|---------|
| SQLCOST014 | 838 | 37 | 20 | 17 | 0 | 0 |
| SQLCOST001 | 341 | 10 | 10 | 0 | 0 | 0 |
| SQLCOST002 | 296 | 18 | 18 | 0 | 0 | 0 |
| SQLCOST015 | 246 | 16 | 16 | 0 | 0 | 0 |
| SQLCOST038 | 347 | 10 | 10 | 0 | 0 | 0 |
| SQLCOST042 | 504 | 10 | 10 | 0 | 0 | 0 |
| SQLCOST007 | 0 | 0 | 0 | 0 | 0 | 0 |
| SQLCOST008 | 94 | 20 | 20 | 0 | 0 | 0 |

`SQLCOST027` remains infrastructure-only. The supplemental parse-marker summary documents top source patterns:

| Source pattern | Count | Signature |
|----------------|-------|-----------|
| `jinja_template` | 4505 | SQL parse failed |
| `dbt_config_wrapper` | 3007 | SQL parse failed |
| `sql_parse_failure` | 135 | SQL parse failed |
| `compiled_sql_parser_gap` | 3 | SQL parse failed |

## Required benchmark add-on review

Generated: 2026-06-18  
Required support-matrix repos: nba-monte-carlo (DuckDB) and Tuva (DuckDB). These repos were reviewed separately so each repo gets up to 100 cost-ranked examples per emitted behavioral rule. Evidence lives in [`tests/benchmarks/evidence/rule_tp_nba_monte_carlo.json`](../../tests/benchmarks/evidence/rule_tp_nba_monte_carlo.json) and [`tests/benchmarks/evidence/rule_tp_tuva.json`](../../tests/benchmarks/evidence/rule_tp_tuva.json).

### NBA Monte Carlo

- **44/44 rules PASS**

| Rule | Total | Examined | TP | Exempt | FP bug | Unknown | Pass reason |
|------|-------|----------|----|--------|--------|---------|-------------|
| SQLCOST012 | 3 | 3 | 3 | 0 | 0 | 0 | fully_examined |
| SQLCOST022 | 3 | 3 | 3 | 0 | 0 | 0 | fully_examined |
| SQLCOST039 | 5 | 5 | 5 | 0 | 0 | 0 | fully_examined |

`SQLCOST027` parse markers are infrastructure-only and excluded from the behavioral pass bar.

### Tuva

- **44/44 rules PASS**

| Rule | Total | Examined | TP | Exempt | FP bug | Unknown | Pass reason |
|------|-------|----------|----|--------|--------|---------|-------------|
| SQLCOST001 | 138 | 100 | 100 | 0 | 0 | 0 | sampled_100_of_138 |
| SQLCOST002 | 5 | 5 | 5 | 0 | 0 | 0 | fully_examined |
| SQLCOST003 | 1 | 1 | 1 | 0 | 0 | 0 | fully_examined |
| SQLCOST004 | 1 | 1 | 1 | 0 | 0 | 0 | fully_examined |
| SQLCOST006 | 16 | 16 | 11 | 5 | 0 | 0 | fully_examined |
| SQLCOST007 | 155 | 100 | 100 | 0 | 0 | 0 | sampled_100_of_155 |
| SQLCOST008 | 270 | 100 | 100 | 0 | 0 | 0 | sampled_100_of_270 |
| SQLCOST009 | 1 | 1 | 1 | 0 | 0 | 0 | fully_examined |
| SQLCOST011 | 1 | 1 | 1 | 0 | 0 | 0 | fully_examined |
| SQLCOST012 | 69 | 69 | 32 | 37 | 0 | 0 | fully_examined |
| SQLCOST013 | 49 | 49 | 49 | 0 | 0 | 0 | fully_examined |
| SQLCOST014 | 141 | 100 | 93 | 7 | 0 | 0 | sampled_100_of_141 |
| SQLCOST015 | 7 | 7 | 7 | 0 | 0 | 0 | fully_examined |
| SQLCOST017 | 19 | 19 | 14 | 5 | 0 | 0 | fully_examined |
| SQLCOST018 | 26 | 26 | 26 | 0 | 0 | 0 | fully_examined |
| SQLCOST020 | 22 | 22 | 22 | 0 | 0 | 0 | fully_examined |
| SQLCOST031 | 5 | 5 | 5 | 0 | 0 | 0 | fully_examined |
| SQLCOST037 | 1 | 1 | 1 | 0 | 0 | 0 | fully_examined |
| SQLCOST039 | 54 | 54 | 54 | 0 | 0 | 0 | fully_examined |
| SQLCOST040 | 1 | 1 | 1 | 0 | 0 | 0 | fully_examined |

`SQLCOST027` parse markers are infrastructure-only and excluded from the behavioral pass bar.

### Tuva supplemental tail review

Evidence lives in [`tests/benchmarks/evidence/rule_tp_tuva_stratified_supplemental.json`](../../tests/benchmarks/evidence/rule_tp_tuva_stratified_supplemental.json). Behavioral supplemental samples have **0 `fp_bug`** and **0 `unknown`**.

| Rule | Tail total | Supplemental examined | TP | Exempt | FP bug | Unknown |
|------|------------|-----------------------|----|--------|--------|---------|
| SQLCOST007 | 55 | 10 | 10 | 0 | 0 | 0 |
| SQLCOST008 | 170 | 14 | 14 | 0 | 0 | 0 |
| SQLCOST014 | 41 | 13 | 10 | 3 | 0 | 0 |

`SQLCOST027` remains infrastructure-only for Tuva. Top source patterns:

| Source pattern | Count | Signature |
|----------------|-------|-----------|
| `dbt_config_wrapper` | 459 | SQL parse failed |
| `jinja_template` | 23 | SQL parse failed |
| `sql_parse_failure` | 2 | SQL parse failed |
