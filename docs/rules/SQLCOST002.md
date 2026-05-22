# SQLCOST002: Repeated JSON extraction

**Severity:** medium  
**Default threshold:** 2 (configurable via `[rules.SQLCOST002].threshold`)

Detects repeated JSON or semi-structured extraction in one SQL file.

## Fix

Materialize extracted fields once in staging when the same payload field is reused.

## Example

```sql
-- Before: same json field extracted multiple times
SELECT
  json_extract_scalar(payload, '$.user_id') AS user_id,
  json_extract_scalar(payload, '$.user_id') AS user_id_copy
FROM events
```

Extract once in a staging model and reference the column downstream.

## Configuration

```toml
[rules.SQLCOST002]
threshold = 3
```
