# SQLCOST038 — Fan-out join on non-unique key

**Severity:** high

Detects equality joins where keys do not cover the joined model's known `unique_key` grain.

## When it fires

- A join has equality predicates, the joined model's `unique_key` is known from project metadata, and the join keys do not include all grain columns.

## Fix

Join on the upstream model's `unique_key` columns or dedupe the driving side first.

```sql
-- prefer
join dim_users u on e.user_id = u.user_id and e.event_date = u.event_date
```
