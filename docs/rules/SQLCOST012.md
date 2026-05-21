# SQLCOST012: Cross join without explicit allow comment

Detects `CROSS JOIN` and comma joins.

Add a predicate, or document intentional use with `costguard: allow cross-join`.
