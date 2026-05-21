# SQLCOST007: ORDER BY in model

Detects `ORDER BY` in downstream models without `LIMIT`.

Remove intermediate sorts unless they are required for deterministic logic.
