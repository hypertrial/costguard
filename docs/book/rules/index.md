# Rule catalog

Generated from `costguard rules --format json`. Regenerate with:

```bash
python3 scripts/generate_rule_docs.py
```

<!-- generated:rules:start -->
| Severity | Rule | Name | Guide |
| --- | --- | --- | --- |
| medium | `SQLCOST001` | SELECT * in non-staging model | [SQLCOST001](../../rules/SQLCOST001.md) |
| medium | `SQLCOST002` | Repeated JSON extraction | [SQLCOST002](../../rules/SQLCOST002.md) |
| medium | `SQLCOST003` | Repeated regex extraction or replacement | [SQLCOST003](../../rules/SQLCOST003.md) |
| high | `SQLCOST004` | Incremental model without unique_key | [SQLCOST004](../../rules/SQLCOST004.md) |
| high | `SQLCOST005` | Incremental model without date or partition predicate | [SQLCOST005](../../rules/SQLCOST005.md) |
| medium | `SQLCOST006` | Unbounded join risk | [SQLCOST006](../../rules/SQLCOST006.md) |
| low | `SQLCOST007` | ORDER BY in model | [SQLCOST007](../../rules/SQLCOST007.md) |
| medium | `SQLCOST008` | Blind SELECT DISTINCT | [SQLCOST008](../../rules/SQLCOST008.md) |
| low | `SQLCOST009` | Repeated normalization expression | [SQLCOST009](../../rules/SQLCOST009.md) |
| high | `SQLCOST010` | Python model row-wise operation | [SQLCOST010](../../rules/SQLCOST010.md) |
| medium | `SQLCOST011` | Source used directly in mart layer | [SQLCOST011](../../rules/SQLCOST011.md) |
| medium | `SQLCOST012` | Cross join without explicit allow comment | [SQLCOST012](../../rules/SQLCOST012.md) |
| medium | `SQLCOST013` | Unpartitioned window function | [SQLCOST013](../../rules/SQLCOST013.md) |
| low | `SQLCOST014` | Repeated CTE reference | [SQLCOST014](../../rules/SQLCOST014.md) |
| medium | `SQLCOST015` | Expensive expression repeated across downstream models | [SQLCOST015](../../rules/SQLCOST015.md) |
| high | `SQLCOST016` | Non-sargable partition or date predicate | [SQLCOST016](../../rules/SQLCOST016.md) |
| medium | `SQLCOST017` | Function-wrapped join key | [SQLCOST017](../../rules/SQLCOST017.md) |
| medium | `SQLCOST018` | UNION instead of UNION ALL | [SQLCOST018](../../rules/SQLCOST018.md) |
| high | `SQLCOST019` | Incremental model reads source without source-side bound | [SQLCOST019](../../rules/SQLCOST019.md) |
| medium | `SQLCOST020` | Exact distinct aggregation on large model | [SQLCOST020](../../rules/SQLCOST020.md) |
| medium | `SQLCOST021` | BigQuery wildcard table scan without suffix bound | [SQLCOST021](../../rules/SQLCOST021.md) |
| medium | `SQLCOST022` | Python model collects warehouse data locally | [SQLCOST022](../../rules/SQLCOST022.md) |
| info | `SQLCOST023` | Scan without dbt manifest | [SQLCOST023](../../rules/SQLCOST023.md) |
| low | `SQLCOST024` | Schema YAML parse failure | [SQLCOST024](../../rules/SQLCOST024.md) |
| low | `SQLCOST025` | dbt_project.yml metadata issue | [SQLCOST025](../../rules/SQLCOST025.md) |
| low | `SQLCOST026` | File skipped during scan | [SQLCOST026](../../rules/SQLCOST026.md) |
| info | `SQLCOST027` | SQL parse failure | [SQLCOST027](../../rules/SQLCOST027.md) |
| info | `SQLCOST045` | Stale dbt manifest | [SQLCOST045](../../rules/SQLCOST045.md) |
| high | `SQLCOST028` | Missing partition or cluster config on large mart model | [SQLCOST028](../../rules/SQLCOST028.md) |
| medium | `SQLCOST029` | Full-refresh-heavy incremental config | [SQLCOST029](../../rules/SQLCOST029.md) |
| high | `SQLCOST030` | Correlated subquery | [SQLCOST030](../../rules/SQLCOST030.md) |
| medium | `SQLCOST031` | Leading-wildcard LIKE predicate | [SQLCOST031](../../rules/SQLCOST031.md) |
| medium | `SQLCOST032` | OR across partition or date predicates | [SQLCOST032](../../rules/SQLCOST032.md) |
| high | `SQLCOST033` | Pattern-matching join predicate | [SQLCOST033](../../rules/SQLCOST033.md) |
| medium | `SQLCOST034` | Scalar subquery in SELECT list | [SQLCOST034](../../rules/SQLCOST034.md) |
| medium | `SQLCOST035` | Cross-catalog join | [SQLCOST035](../../rules/SQLCOST035.md) |
| high | `SQLCOST036` | Row-exploding UNNEST or LATERAL FLATTEN | [SQLCOST036](../../rules/SQLCOST036.md) |
| high | `SQLCOST037` | NOT IN (subquery) | [SQLCOST037](../../rules/SQLCOST037.md) |
| high | `SQLCOST038` | Fan-out join on non-unique key | [SQLCOST038](../../rules/SQLCOST038.md) |
| medium | `SQLCOST039` | Heavily referenced view or ephemeral model | [SQLCOST039](../../rules/SQLCOST039.md) |
| medium | `SQLCOST040` | Table model with date column should be incremental | [SQLCOST040](../../rules/SQLCOST040.md) |
| medium | `SQLCOST041` | Unbounded window frame | [SQLCOST041](../../rules/SQLCOST041.md) |
| medium | `SQLCOST042` | BigQuery model without partition or date filter | [SQLCOST042](../../rules/SQLCOST042.md) |
| medium | `SQLCOST043` | Incremental merge without target pruning | [SQLCOST043](../../rules/SQLCOST043.md) |
| medium | `SQLCOST044` | Recursive CTE | [SQLCOST044](../../rules/SQLCOST044.md) |

## Descriptions

### `SQLCOST001` — SELECT * in non-staging model

**Severity:** medium

Detects SELECT * in downstream dbt models.

Fix guidance: [SQLCOST001.md](../../rules/SQLCOST001.md)

### `SQLCOST002` — Repeated JSON extraction

**Severity:** medium

Detects repeated semi-structured extraction in one file.

Fix guidance: [SQLCOST002.md](../../rules/SQLCOST002.md)

### `SQLCOST003` — Repeated regex extraction or replacement

**Severity:** medium

Detects repeated or excessive regex work.

Fix guidance: [SQLCOST003.md](../../rules/SQLCOST003.md)

### `SQLCOST004` — Incremental model without unique_key

**Severity:** high

Detects dbt incremental models without a unique key.

Fix guidance: [SQLCOST004.md](../../rules/SQLCOST004.md)

### `SQLCOST005` — Incremental model without date or partition predicate

**Severity:** high

Detects incremental models without an obvious pruning predicate.

Fix guidance: [SQLCOST005.md](../../rules/SQLCOST005.md)

### `SQLCOST006` — Unbounded join risk

**Severity:** medium

Detects joins without safe equality predicates.

Fix guidance: [SQLCOST006.md](../../rules/SQLCOST006.md)

### `SQLCOST007` — ORDER BY in model

**Severity:** low

Detects ORDER BY in non-final models without LIMIT.

Fix guidance: [SQLCOST007.md](../../rules/SQLCOST007.md)

### `SQLCOST008` — Blind SELECT DISTINCT

**Severity:** medium

Detects SELECT DISTINCT deduplication.

Fix guidance: [SQLCOST008.md](../../rules/SQLCOST008.md)

### `SQLCOST009` — Repeated normalization expression

**Severity:** low

Detects repeated lower/upper trim normalization.

Fix guidance: [SQLCOST009.md](../../rules/SQLCOST009.md)

### `SQLCOST010` — Python model row-wise operation

**Severity:** high

Detects row-wise pandas patterns in Python dbt models.

Fix guidance: [SQLCOST010.md](../../rules/SQLCOST010.md)

### `SQLCOST011` — Source used directly in mart layer

**Severity:** medium

Detects dbt source() usage in marts.

Fix guidance: [SQLCOST011.md](../../rules/SQLCOST011.md)

### `SQLCOST012` — Cross join without explicit allow comment

**Severity:** medium

Detects CROSS JOIN and comma joins.

Fix guidance: [SQLCOST012.md](../../rules/SQLCOST012.md)

### `SQLCOST013` — Unpartitioned window function

**Severity:** medium

Detects OVER () and window functions without PARTITION BY.

Fix guidance: [SQLCOST013.md](../../rules/SQLCOST013.md)

### `SQLCOST014` — Repeated CTE reference

**Severity:** low

Detects CTEs referenced multiple times downstream.

Fix guidance: [SQLCOST014.md](../../rules/SQLCOST014.md)

### `SQLCOST015` — Expensive expression repeated across downstream models

**Severity:** medium

Detects repeated JSON, regex, or normalization expressions across files.

Fix guidance: [SQLCOST015.md](../../rules/SQLCOST015.md)

### `SQLCOST016` — Non-sargable partition or date predicate

**Severity:** high

Detects filters that wrap likely partition or date columns in functions.

Fix guidance: [SQLCOST016.md](../../rules/SQLCOST016.md)

### `SQLCOST017` — Function-wrapped join key

**Severity:** medium

Detects joins where a join key is transformed inline.

Fix guidance: [SQLCOST017.md](../../rules/SQLCOST017.md)

### `SQLCOST018` — UNION instead of UNION ALL

**Severity:** medium

Detects plain UNION in dbt models.

Fix guidance: [SQLCOST018.md](../../rules/SQLCOST018.md)

### `SQLCOST019` — Incremental model reads source without source-side bound

**Severity:** high

Detects incremental models that read source() before applying a partition predicate.

Fix guidance: [SQLCOST019.md](../../rules/SQLCOST019.md)

### `SQLCOST020` — Exact distinct aggregation on large model

**Severity:** medium

Detects count(distinct ...) in downstream models.

Fix guidance: [SQLCOST020.md](../../rules/SQLCOST020.md)

### `SQLCOST021` — BigQuery wildcard table scan without suffix bound

**Severity:** medium

Detects wildcard tables without a bounded _TABLE_SUFFIX filter.

Fix guidance: [SQLCOST021.md](../../rules/SQLCOST021.md)

### `SQLCOST022` — Python model collects warehouse data locally

**Severity:** medium

Detects Python dbt patterns that pull warehouse data into local memory.

Fix guidance: [SQLCOST022.md](../../rules/SQLCOST022.md)

### `SQLCOST023` — Scan without dbt manifest

**Severity:** info

Reports when Costguard scans dbt metadata from YAML/SQL only without a manifest.

Fix guidance: [SQLCOST023.md](../../rules/SQLCOST023.md)

### `SQLCOST024` — Schema YAML parse failure

**Severity:** low

Reports when a dbt schema YAML file failed to parse.

Fix guidance: [SQLCOST024.md](../../rules/SQLCOST024.md)

### `SQLCOST025` — dbt_project.yml metadata issue

**Severity:** low

Reports when dbt_project.yml failed to parse or has an ambiguous models block.

Fix guidance: [SQLCOST025.md](../../rules/SQLCOST025.md)

### `SQLCOST026` — File skipped during scan

**Severity:** low

Reports when a SQL or dbt file exceeds the configured scan size limit and was not loaded.

Fix guidance: [SQLCOST026.md](../../rules/SQLCOST026.md)

### `SQLCOST027` — SQL parse failure

**Severity:** info

Reports when a dbt model SQL file could not be parsed and rules may fall back to regex heuristics.

Fix guidance: [SQLCOST027.md](../../rules/SQLCOST027.md)

### `SQLCOST045` — Stale dbt manifest

**Severity:** info

Reports when target/manifest.json is older than modified dbt model SQL files.

Fix guidance: [SQLCOST045.md](../../rules/SQLCOST045.md)

### `SQLCOST028` — Missing partition or cluster config on large mart model

**Severity:** high

Detects incremental or table materialized mart models without partition_by or cluster_by.

Fix guidance: [SQLCOST028.md](../../rules/SQLCOST028.md)

### `SQLCOST029` — Full-refresh-heavy incremental config

**Severity:** medium

Detects incremental models configured with full_refresh or sync_all_columns schema changes.

Fix guidance: [SQLCOST029.md](../../rules/SQLCOST029.md)

### `SQLCOST030` — Correlated subquery

**Severity:** high

Detects correlated subqueries in filters or join predicates.

Fix guidance: [SQLCOST030.md](../../rules/SQLCOST030.md)

### `SQLCOST031` — Leading-wildcard LIKE predicate

**Severity:** medium

Detects LIKE/ILIKE patterns that start with % or _ in filters.

Fix guidance: [SQLCOST031.md](../../rules/SQLCOST031.md)

### `SQLCOST032` — OR across partition or date predicates

**Severity:** medium

Detects OR expressions joining predicates on likely partition or date columns.

Fix guidance: [SQLCOST032.md](../../rules/SQLCOST032.md)

### `SQLCOST033` — Pattern-matching join predicate

**Severity:** high

Detects LIKE, RLIKE, or regexp_like predicates in JOIN ON clauses.

Fix guidance: [SQLCOST033.md](../../rules/SQLCOST033.md)

### `SQLCOST034` — Scalar subquery in SELECT list

**Severity:** medium

Detects per-row scalar subqueries in downstream model projections.

Fix guidance: [SQLCOST034.md](../../rules/SQLCOST034.md)

### `SQLCOST035` — Cross-catalog join

**Severity:** medium

Detects joins between fully qualified tables with different catalog parts.

Fix guidance: [SQLCOST035.md](../../rules/SQLCOST035.md)

### `SQLCOST036` — Row-exploding UNNEST or LATERAL FLATTEN

**Severity:** high

Detects UNNEST, LATERAL FLATTEN, or CROSS JOIN UNNEST followed by deduplication or aggregation.

Fix guidance: [SQLCOST036.md](../../rules/SQLCOST036.md)

### `SQLCOST037` — NOT IN (subquery)

**Severity:** high

Detects NOT IN (subquery) anti-join patterns in filters or join predicates.

Fix guidance: [SQLCOST037.md](../../rules/SQLCOST037.md)

### `SQLCOST038` — Fan-out join on non-unique key

**Severity:** high

Detects equality joins where keys do not cover the joined model's known grain.

Fix guidance: [SQLCOST038.md](../../rules/SQLCOST038.md)

### `SQLCOST039` — Heavily referenced view or ephemeral model

**Severity:** medium

Detects view or ephemeral models referenced by many downstream models.

Fix guidance: [SQLCOST039.md](../../rules/SQLCOST039.md)

### `SQLCOST040` — Table model with date column should be incremental

**Severity:** medium

Detects full-rebuild table models with recognized date or partition columns that look append-only.

Fix guidance: [SQLCOST040.md](../../rules/SQLCOST040.md)

### `SQLCOST041` — Unbounded window frame

**Severity:** medium

Detects window functions with ROWS/RANGE BETWEEN UNBOUNDED PRECEDING AND UNBOUNDED FOLLOWING.

Fix guidance: [SQLCOST041.md](../../rules/SQLCOST041.md)

### `SQLCOST042` — BigQuery model without partition or date filter

**Severity:** medium

Detects BigQuery models that read source() or ref() without an obvious partition or date filter.

Fix guidance: [SQLCOST042.md](../../rules/SQLCOST042.md)

### `SQLCOST043` — Incremental merge without target pruning

**Severity:** medium

Detects incremental merge models without incremental_predicates for target-side pruning.

Fix guidance: [SQLCOST043.md](../../rules/SQLCOST043.md)

### `SQLCOST044` — Recursive CTE

**Severity:** medium

Detects WITH RECURSIVE common table expressions.

Fix guidance: [SQLCOST044.md](../../rules/SQLCOST044.md)
<!-- generated:rules:end -->
