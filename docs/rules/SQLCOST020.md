# SQLCOST020: Exact distinct aggregation on large model

**Severity:** medium (high when multiple distincts appear in one query)

Detects `count(distinct ...)` in marts or intermediate models.

Consider pre-aggregating, deduping upstream, or using approximate distinct functions where acceptable.
