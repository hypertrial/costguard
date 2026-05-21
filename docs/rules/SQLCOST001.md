# SQLCOST001: SELECT * in non-staging model

Detects `SELECT *` or `table.*` in downstream models such as marts or intermediate models.

Use explicit column lists to reduce scanned data and schema-drift risk.
