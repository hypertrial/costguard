# SQLCOST005: Incremental model without date or partition predicate

**Severity:** high

Detects incremental models using `is_incremental()` without an obvious bounded read predicate.

## Accepted pruning signals

Inside the incremental block:

- date or partition columns such as `updated_at`, `event_date`, `_PARTITIONDATE`, `block_time`, `evt_block_time`, `block_date`, `block_timestamp`
- block identifiers such as `block_number`, `block_num`, `evt_block_number`
- time bucketing such as `date_trunc(`, `minute`, `hour`
- key-bounded anti-join patterns such as `where id not in (select id from {{ this }})`
- max-key lookups such as `> (select max(updated_at) from {{ this }})`

## Fix

Add a bounded predicate to limit incremental reads when none of the above is present.

## Suppression

```sql
-- costguard: disable-next-line=SQLCOST005
SELECT * FROM {{ this }} WHERE 1 = 1
```

Use sparingly — unbounded incrementals often cause full table scans every run.

## Example

```sql
{% if is_incremental() %}
  WHERE block_time >= (SELECT max(block_time) FROM {{ this }})
{% endif %}
```
