# SQLCOST006: Unbounded join risk

Detects joins without clear equality predicates.

Join on stable keys before applying transformations where possible. Function-wrapped join keys are reported separately by `SQLCOST017`.
