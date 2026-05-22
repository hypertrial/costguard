# SQLCOST012: Cross join without explicit allow comment

**Severity:** high

Detects `CROSS JOIN` and comma joins without a documented allow comment.

## Fix

Add a join predicate, or document intentional use:

```sql
-- costguard: allow cross-join
SELECT a.id, b.id
FROM a
CROSS JOIN b
```

Both `-- costguard: allow cross-join` and bare `costguard: allow cross-join` are accepted.

See [Suppressions](../book/reference/suppressions.md).
