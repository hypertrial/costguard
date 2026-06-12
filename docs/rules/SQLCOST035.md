# SQLCOST035 — Cross-catalog join

**Severity:** medium

Detects joins between fully qualified tables whose catalog parts differ.

## When it fires

- Both sides of a join use three-part names (`catalog.schema.table`).
- The first (catalog) segment differs between the joined relations.

## Warehouse scope

Trino and Databricks.

## Fix

Stage remote catalog data locally or consolidate joins within one catalog when possible.

```sql
-- avoid when catalogs differ
from hive.default.orders o
join iceberg.analytics.users u on o.id = u.id
```
