# SQLCOST028 — Missing partition or cluster config on large mart model

**Severity:** high

Detects incremental or table-materialized dbt models in mart paths without `partition_by` or `cluster_by` configuration.

## When it fires

- Model path is under `marts/` or `mart/`.
- `materialized` is `incremental` or `table`.
- Neither `partition_by` nor `cluster_by` is set in inline config, schema YAML, folder defaults, or manifest metadata.

## Warehouse scope

BigQuery, Snowflake, and Databricks.

## Fix

Add warehouse-native partition or cluster hints so large marts prune scans and colocate related rows.

```yaml
models:
  - name: fct_orders
    config:
      materialized: incremental
      unique_key: order_id
      partition_by:
        field: order_date
        data_type: date
      cluster_by: [customer_id]
```
