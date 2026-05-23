# SQLCOST012: Cross join without explicit allow comment

**Severity:** high

Detects `CROSS JOIN` and comma joins without a documented allow comment.

`CROSS JOIN UNNEST(...)`, `CROSS JOIN TABLE(...)`, and similar table-function cross joins are **intentional Trino idioms** and are not flagged.

## Fix

Add a join predicate, or document intentional use:

```sql
-- costguard: allow cross-join
SELECT a.id, b.id
FROM a
CROSS JOIN b
```

Both `-- costguard: allow cross-join` and bare `costguard: allow cross-join` are accepted.

Regex-only comma/cross join detection emits `confidence: low`. Use `--min-confidence high` to suppress regex-only hits in PR gates.

See [Suppressions](../book/reference/suppressions.md).
