# SQLCOST037 — NOT IN (subquery)

**Severity:** high

Detects `NOT IN (subquery)` anti-join patterns in filters or join predicates.

## When it fires

- A `NOT IN` expression wraps a subquery in `WHERE` or `JOIN ON`.

## Fix

Rewrite as `NOT EXISTS`, `LEFT JOIN ... IS NULL`, or pre-filter the driving set.

```sql
-- prefer
where not exists (
  select 1 from excluded e where e.id = t.id
)
```
