# SQLCOST032 — OR across partition or date predicates

**Severity:** medium

Detects `OR` expressions that combine predicates on likely partition or date columns.

## When it fires

- Both sides of an `OR` reference partition-like columns such as `event_date`, `block_time`, or `_PARTITIONTIME`.

## Fix

Split into `UNION ALL` branches or rewrite to a single bounded predicate on one partition column.

```sql
-- prefer
where event_date in (current_date, current_date - 1)
-- or union branches with independent partition bounds
```
