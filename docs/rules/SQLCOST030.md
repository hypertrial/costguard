# SQLCOST030 — Correlated subquery

**Severity:** high

Detects correlated subqueries in `WHERE` clauses or `JOIN ON` predicates — subqueries that reference columns from an outer query scope.

## When it fires

- `EXISTS (...)` or comparison/`IN` subqueries whose inner query references outer table aliases.
- Applies in downstream SQL models after AST extraction, with regex fallback when parsing fails.

## Fix

Rewrite correlated filters as joins or pre-filter the driving table before the lookup.

```sql
-- prefer
select o.id
from orders o
join line_items li on li.order_id = o.id
group by 1
```
