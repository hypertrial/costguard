# SQLCOST039 — Heavily referenced view or ephemeral model

**Severity:** medium

Detects `view` or `ephemeral` models referenced by many downstream models.

## When it fires

- Model `materialized` is `view` or `ephemeral` and downstream `ref()` count meets the configured threshold (default 4).

## Fix

Materialize as `table` or `incremental` when the model is reused across the DAG.

```yaml
models:
  - name: int_orders_enriched
    config:
      materialized: table
```
