# SQLCOST043 — Incremental merge without target pruning

**Severity:** medium

Detects Snowflake incremental `merge` models without `incremental_predicates` for target-side pruning.

## When it fires

- Model is `incremental` with `incremental_strategy='merge'` and no `incremental_predicates` in config or SQL.

## Fix

Add `incremental_predicates` in model config to prune `DBT_INTERNAL_DEST` on merge.

```sql
{{
  config(
    materialized='incremental',
    unique_key='id',
    incremental_strategy='merge',
    incremental_predicates=['DBT_INTERNAL_DEST.event_date >= dateadd(day, -3, current_date())']
  )
}}
```
