# SQLCOST006: Unbounded join risk

Detects joins without clear equality predicates or with function-wrapped join keys.

Join on stable keys before applying transformations where possible.
