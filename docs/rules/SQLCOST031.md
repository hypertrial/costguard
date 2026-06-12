# SQLCOST031 — Leading-wildcard LIKE predicate

**Severity:** medium

Detects `LIKE` / `ILIKE` patterns that begin with `%` or `_` in filters (`WHERE` or `JOIN ON`).

## When it fires

- Predicate uses a leading wildcard, e.g. `email LIKE '%@example.com'`.

## Fix

Use prefix search, tokenized lookup tables, or warehouse search indexes instead of `%value` patterns.

```sql
-- prefer prefix patterns when possible
where email like 'user%'
```
