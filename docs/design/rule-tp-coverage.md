# Rule TP coverage (all benchmark repos)

Generated: 2026-06-16  
Corpus: spellbook (Trino), jaffle-shop (duckdb), mattermost-warehouse (Snowflake, raw), data-infra (BigQuery, raw).  
Pass bar: **20 recorded TP examples** OR **100% clean** (zero FP and zero unknown buckets).  
Tool: [`scripts/rule_tp_census.py`](../../scripts/rule_tp_census.py); evidence: [`tests/benchmarks/rule_tp_evidence.json`](../../tests/benchmarks/rule_tp_evidence.json).

For the full manual review workflow (adjudication loop, registry contracts, bucket taxonomy), see [Manual rule review playbook](manual-rule-review.md).

## Verification methodology

Each rule was validated by:

1. Scanning all four benchmark repos at pinned commits ([`tests/benchmarks/repos.toml`](../../tests/benchmarks/repos.toml)) with compiled manifests where available.
2. Classifying every diagnostic into **TP**, **FP**, or **unknown** using [`scripts/bucket_rule_diagnostics.py`](../../scripts/bucket_rule_diagnostics.py) buckets and [`tests/benchmarks/fp_registry.toml`](../../tests/benchmarks/fp_registry.toml) verdicts (same logic as [`scripts/precision_triage.py`](../../scripts/precision_triage.py)).
3. Recording up to 20 TP snippets per rule in evidence JSON for spot-check audits.
4. Applying the pass bar: ≥20 TP **or** fully clean; infrastructure rules (`SQLCOST023`–`027`) excluded.

Rules with **zero findings** in the four-repo scan pass as `vacuous_clean` when corpus `expect_rules` / `forbid_rules` and registry buckets prove the rule is wired correctly (e.g. SQLCOST021, SQLCOST043).

## How to reproduce

```bash
# Requires cached checkouts (~/.cache/costguard/benchmarks/) and release costguard binary
python3 scripts/rule_tp_census.py \
  --repos spellbook jaffle-shop mattermost-warehouse data-infra \
  --emit-evidence

# Force dbt recompile if manifest stale
python3 scripts/rule_tp_census.py --force-compile --emit-evidence
```

Exit code `0` means 44/44 PASS. Refresh this doc’s scoreboard after intentional rule or registry changes.

## Summary
- **44/44 rules PASS**

| Rule | Total | TP | FP | Unknown | Pass reason | Rationale |
|------|-------|----|----|---------|-------------|-----------|
| SQLCOST001 | 441 | 441 | 0 | 0 | tp>=20 | SELECT * in downstream marts widens scans and hides schema drift. |
| SQLCOST002 | 396 | 396 | 0 | 0 | tp>=20 | Repeated JSON extraction in one model is redundant work at scale. |
| SQLCOST003 | 32 | 32 | 0 | 0 | tp>=20 | Repeated regex calls in one model add CPU-heavy per-row work. |
| SQLCOST004 | 60 | 60 | 0 | 0 | tp>=20 | Incremental merge/append without unique_key risks bad merges and full rewrites. |
| SQLCOST005 | 2 | 2 | 0 | 0 | clean | Incremental models without partition/date predicates scan unbounded history. |
| SQLCOST006 | 135 | 93 | 42 | 0 | tp>=20_with_residual_fp | Non-equality joins can explode row counts; equality joins on macros/templates skipped. |
| SQLCOST007 | 58 | 58 | 0 | 0 | tp>=20 | ORDER BY without LIMIT in intermediate models adds avoidable sorts. |
| SQLCOST008 | 460 | 454 | 0 | 6 | tp>=20_with_residual_fp | Blind DISTINCT hides upstream dupes; skipped when GROUP BY deduplicates. |
| SQLCOST009 | 0 | 0 | 0 | 0 | vacuous_clean | Cost signal validated against registry buckets. |
| SQLCOST010 | 0 | 0 | 0 | 0 | vacuous_clean | Cost signal validated against registry buckets. |
| SQLCOST011 | 0 | 0 | 0 | 0 | vacuous_clean | Cost signal validated against registry buckets. |
| SQLCOST012 | 40 | 20 | 20 | 0 | tp>=20_with_residual_fp | Cross/comma joins multiply rows; CTE broadcast and UNNEST patterns exempt. |
| SQLCOST013 | 223 | 223 | 0 | 0 | tp>=20 | Window functions without PARTITION BY force global sorts. |
| SQLCOST014 | 612 | 594 | 18 | 0 | tp>=20_with_residual_fp | Repeated CTE references may recompute expensive branches; macros skipped. |
| SQLCOST015 | 346 | 338 | 0 | 8 | tp>=20_with_residual_fp | Cross-file expensive expression reuse indicates project-level waste. |
| SQLCOST016 | 0 | 0 | 0 | 0 | vacuous_clean | Function-wrapped partition columns block pruning; bounded date()/date_trunc exempt. |
| SQLCOST017 | 3 | 3 | 0 | 0 | clean | Function-wrapped join keys block hash joins; time-bucket/coalesce/normalize joins exempt. |
| SQLCOST018 | 378 | 378 | 0 | 0 | tp>=20 | UNION dedup is costlier than UNION ALL for stacked models. |
| SQLCOST019 | 67 | 67 | 0 | 0 | tp>=20 | Incremental reads of sources without local partition bounds. |
| SQLCOST020 | 22 | 22 | 0 | 0 | tp>=20 | COUNT(DISTINCT) on large downstream models is expensive exact dedup. |
| SQLCOST021 | 0 | 0 | 0 | 0 | vacuous_clean | BigQuery wildcard scans without suffix bounds (vacuous in current corpus). |
| SQLCOST022 | 0 | 0 | 0 | 0 | vacuous_clean | dbt Python models materialized as tables (vacuous in current corpus). |
| SQLCOST023 | 2 | 0 | 0 | 2 | infrastructure_na | Infrastructure: scan without manifest (N/A). |
| SQLCOST024 | 0 | 0 | 0 | 0 | infrastructure_na | Infrastructure: schema YAML parse failure (N/A). |
| SQLCOST025 | 0 | 0 | 0 | 0 | infrastructure_na | Infrastructure: dbt_project.yml metadata (N/A). |
| SQLCOST026 | 1 | 0 | 0 | 1 | infrastructure_na | Infrastructure: skipped oversized files (N/A). |
| SQLCOST027 | 598 | 0 | 0 | 598 | infrastructure_na | Infrastructure: SQL parse failure marker (N/A). |
| SQLCOST028 | 171 | 171 | 0 | 0 | tp>=20 | BigQuery/Snowflake marts missing partition_by/cluster_by. |
| SQLCOST029 | 114 | 114 | 0 | 0 | tp>=20 | Full-refresh-heavy incremental configs on large models. |
| SQLCOST030 | 7 | 7 | 0 | 0 | clean | Correlated subqueries execute per outer row. |
| SQLCOST031 | 15 | 15 | 0 | 0 | clean | Leading-wildcard LIKE prevents index/partition pruning. |
| SQLCOST032 | 0 | 0 | 0 | 0 | vacuous_clean | OR across partition predicates forces multi-scan plans. |
| SQLCOST033 | 7 | 7 | 0 | 0 | clean | Pattern-matching join predicates devolve to nested loops. |
| SQLCOST034 | 0 | 0 | 0 | 0 | vacuous_clean | Scalar subqueries in SELECT run per output row. |
| SQLCOST035 | 0 | 0 | 0 | 0 | vacuous_clean | Cross-catalog joins add remote IO (vacuous after subquery fix). |
| SQLCOST036 | 193 | 193 | 0 | 0 | tp>=20 | UNNEST/FLATTEN plus dedup implies row explosion then shrink. |
| SQLCOST037 | 78 | 78 | 0 | 0 | tp>=20 | NOT IN (subquery) anti-joins are expensive and NULL-unsafe. |
| SQLCOST038 | 2021 | 2021 | 0 | 0 | tp>=20 | Fan-out joins on non-unique dimension keys multiply rows. |
| SQLCOST039 | 71 | 71 | 0 | 0 | tp>=20 | Heavily referenced views/ephemeral models amplify downstream cost. |
| SQLCOST040 | 12 | 12 | 0 | 0 | clean | Append-only table models with date columns should be incremental. |
| SQLCOST041 | 62 | 62 | 0 | 0 | tp>=20 | Unbounded window frames force full-partition scans. |
| SQLCOST042 | 604 | 604 | 0 | 0 | tp>=20 | BigQuery models reading refs/sources without partition filters. |
| SQLCOST043 | 0 | 0 | 0 | 0 | vacuous_clean | Snowflake merge without target pruning (vacuous in mattermost corpus). |
| SQLCOST044 | 6 | 6 | 0 | 0 | clean | Recursive CTEs can iterate unpredictably on large graphs. |

## Platform-specific notes
- **SQLCOST021** (BigQuery wildcard): no findings in data-infra scan; recall corpus proves rule fires.
- **SQLCOST042** (BigQuery partition filter): 604 TP examples on data-infra.
- **SQLCOST043** (Snowflake merge pruning): no findings on mattermost-warehouse; recall corpus covers.
- **SQLCOST028** (partition/cluster config): 171 TP on data-infra BigQuery marts.

## Vacuous and infrastructure rules

| Pass reason | Meaning | Examples |
| --- | --- | --- |
| `vacuous_clean` | No findings in four-repo scan; corpus + registry prove rule behavior | SQLCOST009–011, SQLCOST021–022, SQLCOST032–035, SQLCOST043 |
| `infrastructure_na` | SQLCOST023–027 — scan/parse/metadata markers, not cost anti-patterns | SQLCOST027 parse-failure markers counted but not behaviorally adjudicated |
| `clean` | Fewer than 20 findings but 0 FP and 0 unknown | SQLCOST005, SQLCOST017, SQLCOST030–031, SQLCOST033, SQLCOST040, SQLCOST044 |

## Residual FP and unknown (pass via TP≥20)

Rules may PASS without zero FPs. Known debt as of 2026-06-16:

| Rule | FP | Unknown | Likely buckets / follow-up |
| --- | --- | --- | --- |
| SQLCOST006 | 42 | 0 | `equality_join` on macro/Jinja SQL — extend macro skip or raw `ON` fallback |
| SQLCOST008 | 0 | 6 | `blind_distinct` — extend GROUP BY dedup detection or add registry rows |
| SQLCOST012 | 20 | 0 | Residual FP buckets after CTE-broadcast fix — inspect via `bucket_rule_diagnostics.py --rule SQLCOST012` |
| SQLCOST014 | 18 | 0 | `other` on macro/homonym CTE templates — `/macros/` skip reduced but did not eliminate |
| SQLCOST015 | 0 | 8 | Unclassified buckets — extend `classify_sqlcost015` or add registry TP/FP rows |

Paydown workflow: [Manual rule review playbook — Residual FP policy](manual-rule-review.md#residual-fp-policy).
