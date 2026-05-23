# SQLCOST019: Incremental model reads source without source-side bound

**Severity:** high

Detects incremental dbt models that use `source()` but only filter after transformations, or lack an early source date/partition predicate.

**Exemptions:** partition predicates anywhere in the `source()` read scope (including `JOIN ON`), and bounded predicates inside source CTEs.

Push the partition or date predicate into the first CTE or subquery that reads `source()`.
