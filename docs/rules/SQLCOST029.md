# SQLCOST029 — Full-refresh-heavy incremental config

**Severity:** medium

Detects incremental dbt models configured to rebuild aggressively via `full_refresh: true` or `on_schema_change: sync_all_columns`.

## When it fires

- `materialized: incremental`.
- `full_refresh: true`, or `on_schema_change: sync_all_columns`.

## Fix

Prefer bounded incremental merges in production marts. Use `append_new_columns` or fail on unexpected schema drift unless a full rebuild is intentional.

```sql
{{ config(
    materialized='incremental',
    unique_key='id',
    on_schema_change='append_new_columns'
) }}
```
