# SQLCOST042 — BigQuery model without partition or date filter

**Severity:** medium

Detects BigQuery models that read `source()` or `ref()` without an obvious partition or date filter.

## When it fires

- BigQuery platform, model references `source()` or `ref()`, and no recognized partition/date predicate is present.

## Fix

Add `_PARTITIONDATE`, `_PARTITIONTIME`, or `event_date` filters before downstream joins.

```sql
select *
from {{ ref('stg_events') }}
where _partitiondate >= date_sub(current_date(), interval 3 day)
```
