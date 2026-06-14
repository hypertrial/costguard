# SQLCOST044 — Recursive CTE

**Severity:** medium

Detects `WITH RECURSIVE` common table expressions.

## When it fires

- SQL contains a recursive CTE definition.

## Fix

Cap recursion depth, pre-materialize graph edges, or use iterative batching when possible.

```sql
-- prefer bounded recursion or staged graph tables
with recursive graph as (
  select parent_id, child_id, 1 as depth
  from edges
  where parent_id = 1
  union all
  select e.parent_id, e.child_id, g.depth + 1
  from edges e
  join graph g on e.parent_id = g.child_id
  where g.depth < 10
)
select * from graph
```
