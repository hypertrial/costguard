# SQLCOST004: Incremental model without unique_key

**Severity:** high

Detects dbt incremental models without a configured `unique_key`.

## Config sources

Config can come from:

- inline SQL `{{ config(...) }}`
- schema/properties YAML (`models[].config.unique_key`)
- nested `dbt_project.yml` folder defaults
- compiled manifest metadata

## Exemptions

Append-only incrementals are exempt when `incremental_strategy: append` is set explicitly in SQL or YAML config.

## Fix

Merge/update incremental models should define stable keys unless they intentionally use append strategy.

## Example

```yaml
# schema.yml
models:
  - name: fct_orders
    config:
      materialized: incremental
      unique_key: order_id
      incremental_strategy: merge
```
