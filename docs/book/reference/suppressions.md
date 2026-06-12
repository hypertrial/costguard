# Suppressions

Place Costguard directives in SQL or Python model comments. Both `-- costguard: ...` (SQL style) and bare `costguard: ...` comments are accepted.

## Directives

| Directive | Scope | Example |
| --- | --- | --- |
| `disable-next-line=RULE` | Next line only | `-- costguard: disable-next-line=SQLCOST005` |
| `disable=RULE` | Current line | `-- costguard: disable=SQLCOST001` |
| `disable-file=RULE` | Entire file | `-- costguard: disable-file=SQLCOST002` |
| `allow cross-join` | Cross join on marked line | `-- costguard: allow cross-join` |

Multiple rules can be comma-separated: `disable-next-line=SQLCOST001,SQLCOST002`.

## SQL example

```sql
-- costguard: disable-next-line=SQLCOST005
SELECT *
FROM incremental_model
WHERE 1 = 1
```

## Cross join (SQLCOST012)

```sql
-- costguard: allow cross-join
SELECT a.id, b.id
FROM a
CROSS JOIN b
```

Bare `costguard: allow cross-join` (without `--`) also works.

## Guidance

- Suppress only intentional exceptions with documented rationale.
- Prefer fixing the underlying cost/perf issue when possible.
- See [PR check workflow](../../design/pr-check-primary-workflow.md) for product-level suppression guidance.

## Related

- [SQLCOST012](../../rules/SQLCOST012.md)
- [Rule catalog](../rules/index.md)
