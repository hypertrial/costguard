# SQLCOST014: Repeated CTE reference

Detects CTEs referenced multiple times downstream in one query.

Materialize expensive or heavily reused CTEs when the warehouse may recompute them.
