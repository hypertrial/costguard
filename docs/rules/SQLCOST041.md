# SQLCOST041 — Unbounded window frame

**Severity:** medium

Detects window functions with `ROWS`/`RANGE BETWEEN UNBOUNDED PRECEDING AND UNBOUNDED FOLLOWING`.

## When it fires

- A window function specifies an unbounded frame on both start and end bounds.

## Fix

Bound the frame with a finite `ROWS`/`RANGE` window when cumulative logic allows it.

```sql
-- prefer
sum(amount) over (
  partition by user_id
  order by event_time
  rows between 29 preceding and current row
)
```
