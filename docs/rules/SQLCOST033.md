# SQLCOST033 — Pattern-matching join predicate

**Severity:** high

Detects `LIKE`, `RLIKE`, `REGEXP`, or `regexp_like` predicates inside `JOIN ON` clauses.

## When it fires

- Join equality or filter logic uses pattern matching instead of stable keys.

## Fix

Normalize join keys upstream and join on stored tokenized columns with equality predicates.

```sql
-- prefer
join tokenized_users u on a.email_norm = u.email_norm
```
