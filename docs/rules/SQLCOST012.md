# SQLCOST012: Cross join without explicit allow comment

**Severity:** medium

Detects `CROSS JOIN` and comma joins without a documented allow comment.

`CROSS JOIN UNNEST(...)`, `CROSS JOIN TABLE(...)`, and similar table-function cross joins are **intentional Trino idioms** and are not flagged.

Cross joins to date-spine CTEs such as `check_date`, `time_seq`, or `date_ranges` are **intentional** for incremental window scans.

Comma-join detection ignores commas in `GROUP BY`/`ORDER BY`, inside `ARRAY[...]`, and after explicit `JOIN` keywords. Jinja `{# ... #}` comments are masked so embedded `from` text does not false-trigger.

## Fix

Add a join predicate, or suppress intentional use on the line above:

```sql
-- costguard: disable-next-line=SQLCOST012
SELECT a.id, b.id
FROM a
CROSS JOIN b
```

Both `-- costguard: disable-next-line=SQLCOST012` and bare `costguard: disable-next-line=SQLCOST012` are accepted.

Regex-only comma/cross join detection emits `confidence: low`. Parsed comma joins emit `confidence: medium` (regex-derived even when the file parses). AST-confirmed `CROSS JOIN` emits `confidence: high`. Use `--min-confidence high` (and optionally `--min-confidence-filter`) to hide lower-confidence hits in PR gates.

See [Suppressions](../book/reference/suppressions.md).
