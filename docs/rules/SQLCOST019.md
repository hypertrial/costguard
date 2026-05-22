# SQLCOST019: Incremental model reads source without source-side bound

**Severity:** high

Detects incremental dbt models that use `source()` but only filter after transformations, or lack an early source date/partition predicate.

Push the partition or date predicate into the first CTE or subquery that reads `source()`.
