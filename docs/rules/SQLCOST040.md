# SQLCOST040 — Table model with date column should be incremental

**Severity:** medium

Detects full-rebuild `table` models with recognized date or partition columns that look append-only.

## When it fires

- Downstream model is materialized as `table`, does not use `is_incremental()`, and SQL references recognized date/partition columns.

## Fix

Switch to incremental materialization with a bounded date or partition predicate.

```sql
{{ config(materialized='incremental', unique_key='id') }}
select id, event_date from events
{% if is_incremental() %}
where event_date >= current_date - 3
{% endif %}
```
