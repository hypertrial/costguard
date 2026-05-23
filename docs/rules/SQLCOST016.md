# SQLCOST016: Non-sargable partition or date predicate

**Severity:** high

Detects filters that wrap likely partition or date columns in functions, such as `date(block_time)`, `cast(event_time as date)`, or `date_trunc('day', created_at)`.

**Exemptions:** bounded `date_trunc` comparisons on partition columns, and partition-wrap filters in staging models.

Compare the raw column to a bounded range instead:

```sql
where block_time >= current_date
  and block_time < current_date + interval '1 day'
```
