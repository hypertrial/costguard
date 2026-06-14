# SQLCOST036 — Row-exploding UNNEST or LATERAL FLATTEN

**Severity:** high

Detects `UNNEST`, `LATERAL FLATTEN`, or `CROSS JOIN UNNEST` followed by deduplication or aggregation.

## When it fires

- A row-exploding table factor is present and the model also uses `SELECT DISTINCT` or `COUNT(DISTINCT ...)`.

## Fix

Pre-filter arrays before exploding, or materialize exploded rows once upstream.

```sql
-- prefer
with exploded as (
  select id, value
  from events, unnest(payload.items) as value
  where event_date >= current_date - 3
)
select id, count(distinct value) from exploded group by 1
```
