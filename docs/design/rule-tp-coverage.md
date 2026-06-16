# Rule TP coverage (all benchmark repos)

Generated: 2026-06-16  
Corpus: spellbook (Trino), jaffle-shop (duckdb), mattermost-warehouse (Snowflake, raw), data-infra (BigQuery, raw).  
Pass bar: **≤100 cost-ranked examples per rule** (100% if fewer) with **0 `fp_bug` + 0 `unknown`** in the examined sample.  
Tool: [`scripts/rule_tp_census.py`](../../scripts/rule_tp_census.py); evidence: [`tests/benchmarks/rule_tp_evidence.json`](../../tests/benchmarks/rule_tp_evidence.json).

For the full manual review workflow (adjudication loop, registry contracts, bucket taxonomy), see [Manual rule review playbook](manual-rule-review.md).

## Verification methodology

Each rule was validated by:

1. Scanning all four benchmark repos at pinned commits ([`tests/benchmarks/repos.toml`](../../tests/benchmarks/repos.toml)) with compiled manifests where available and cost enabled for ranking.
2. Classifying every diagnostic into **TP**, **exempt**, **fp_bug**, or **unknown** using [`scripts/bucket_rule_diagnostics.py`](../../scripts/bucket_rule_diagnostics.py) buckets, [`tests/benchmarks/fp_registry.toml`](../../tests/benchmarks/fp_registry.toml) verdicts, and optional `class` on `verdict = "fp"` rows.
3. Cost-ranking findings by `savings_p50_usd_per_month` and examining up to 100 per rule (100% when total &lt;100).
4. Applying the pass bar: `fp_bug == 0` and `unknown == 0` in the examined sample; infrastructure rules (`SQLCOST023`–`027`) excluded.

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
```

Exit code `0` means 44/44 PASS. Refresh this doc's scoreboard after intentional rule or registry changes.

## Summary
- **44/44 rules PASS**

| Rule | Total | Examined | TP | Exempt | FP bug | Unknown | Pass reason |
|------|-------|----------|----|--------|--------|---------|-------------|
| SQLCOST001 | 441 | 100 | 100 | 0 | 0 | 0 | sampled_100_of_441 |
| SQLCOST002 | 396 | 100 | 100 | 0 | 0 | 0 | sampled_100_of_396 |
| SQLCOST003 | 32 | 32 | 32 | 0 | 0 | 0 | fully_examined |
| SQLCOST004 | 60 | 60 | 60 | 0 | 0 | 0 | fully_examined |
| SQLCOST005 | 2 | 2 | 2 | 0 | 0 | 0 | fully_examined |
| SQLCOST006 | 99 | 99 | 67 | 32 | 0 | 0 | fully_examined |
| SQLCOST007 | 58 | 58 | 58 | 0 | 0 | 0 | fully_examined |
| SQLCOST008 | 443 | 100 | 100 | 0 | 0 | 0 | sampled_100_of_443 |
| SQLCOST009 | 0 | 0 | 0 | 0 | 0 | 0 | vacuous_clean |
| SQLCOST010 | 0 | 0 | 0 | 0 | 0 | 0 | vacuous_clean |
| SQLCOST011 | 0 | 0 | 0 | 0 | 0 | 0 | vacuous_clean |
| SQLCOST012 | 40 | 40 | 20 | 20 | 0 | 0 | fully_examined |
| SQLCOST013 | 223 | 100 | 100 | 0 | 0 | 0 | sampled_100_of_223 |
| SQLCOST014 | 602 | 100 | 96 | 4 | 0 | 0 | sampled_100_of_602 |
| SQLCOST015 | 346 | 100 | 100 | 0 | 0 | 0 | sampled_100_of_346 |
| SQLCOST016 | 0 | 0 | 0 | 0 | 0 | 0 | vacuous_clean |
| SQLCOST017 | 3 | 3 | 3 | 0 | 0 | 0 | fully_examined |
| SQLCOST018 | 378 | 100 | 100 | 0 | 0 | 0 | sampled_100_of_378 |
| SQLCOST019 | 67 | 67 | 67 | 0 | 0 | 0 | fully_examined |
| SQLCOST020 | 22 | 22 | 22 | 0 | 0 | 0 | fully_examined |
| SQLCOST021 | 0 | 0 | 0 | 0 | 0 | 0 | vacuous_clean |
| SQLCOST022 | 0 | 0 | 0 | 0 | 0 | 0 | vacuous_clean |
| SQLCOST023 | 2 | 2 | 0 | 0 | 0 | 2 | infrastructure_na |
| SQLCOST024 | 0 | 0 | 0 | 0 | 0 | 0 | infrastructure_na |
| SQLCOST025 | 0 | 0 | 0 | 0 | 0 | 0 | infrastructure_na |
| SQLCOST026 | 1 | 1 | 0 | 0 | 0 | 1 | infrastructure_na |
| SQLCOST027 | 598 | 100 | 0 | 0 | 0 | 100 | infrastructure_na |
| SQLCOST028 | 171 | 100 | 100 | 0 | 0 | 0 | sampled_100_of_171 |
| SQLCOST029 | 114 | 100 | 100 | 0 | 0 | 0 | sampled_100_of_114 |
| SQLCOST030 | 7 | 7 | 7 | 0 | 0 | 0 | fully_examined |
| SQLCOST031 | 15 | 15 | 15 | 0 | 0 | 0 | fully_examined |
| SQLCOST032 | 0 | 0 | 0 | 0 | 0 | 0 | vacuous_clean |
| SQLCOST033 | 7 | 7 | 7 | 0 | 0 | 0 | fully_examined |
| SQLCOST034 | 0 | 0 | 0 | 0 | 0 | 0 | vacuous_clean |
| SQLCOST035 | 0 | 0 | 0 | 0 | 0 | 0 | vacuous_clean |
| SQLCOST036 | 193 | 100 | 100 | 0 | 0 | 0 | sampled_100_of_193 |
| SQLCOST037 | 78 | 78 | 78 | 0 | 0 | 0 | fully_examined |
| SQLCOST038 | 2021 | 100 | 100 | 0 | 0 | 0 | sampled_100_of_2021 |
| SQLCOST039 | 71 | 71 | 71 | 0 | 0 | 0 | fully_examined |
| SQLCOST040 | 12 | 12 | 12 | 0 | 0 | 0 | fully_examined |
| SQLCOST041 | 62 | 62 | 62 | 0 | 0 | 0 | fully_examined |
| SQLCOST042 | 604 | 100 | 100 | 0 | 0 | 0 | sampled_100_of_604 |
| SQLCOST043 | 0 | 0 | 0 | 0 | 0 | 0 | vacuous_clean |
| SQLCOST044 | 6 | 6 | 6 | 0 | 0 | 0 | fully_examined |

## Platform-specific notes
- **SQLCOST021** (BigQuery wildcard): no findings in data-infra scan; recall corpus proves rule fires.
- **SQLCOST042** (BigQuery partition filter): 604 TP examples on data-infra (100 examined in sample).
- **SQLCOST043** (Snowflake merge pruning): no findings on mattermost-warehouse; recall corpus covers.
- **SQLCOST028** (partition/cluster config): 171 TP on data-infra BigQuery marts.

## Vacuous and infrastructure rules

| Pass reason | Meaning | Examples |
| --- | --- | --- |
| `vacuous_clean` | No findings in four-repo scan; corpus + registry prove rule behavior | SQLCOST009–011, SQLCOST021–022, SQLCOST032–035, SQLCOST043 |
| `infrastructure_na` | SQLCOST023–027 — scan/parse/metadata markers, not cost anti-patterns | SQLCOST027 parse-failure markers counted but not behaviorally adjudicated |
| `fully_examined` | Fewer than 100 total findings; 100% of findings examined | SQLCOST005, SQLCOST017, SQLCOST030–031, SQLCOST033, SQLCOST040, SQLCOST044 |
| `sampled_100_of_N` | Cost-ranked top 100 of N total findings examined | SQLCOST001, SQLCOST038, SQLCOST042 |

## Documented exemptions in sample

Rules may PASS with `exempt > 0` when registry rows use `class = "exempt"` for repo-specific-valid patterns:

| Rule | Exempt (in sample) | Typical buckets |
| --- | --- | --- |
| SQLCOST006 | 32 | `equality_join` (subquery joins with distant `ON` clauses) |
| SQLCOST012 | 20 | `cross_join_unnest`, `date_spine_cross_join`, `group_by_comma_fp`, … |
| SQLCOST014 | 4 | `other` (single-use CTE homonyms) |

Paydown workflow: [Manual rule review playbook — Residual FP policy](manual-rule-review.md#residual-fp-policy).
