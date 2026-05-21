# SQLCOST008: Blind SELECT DISTINCT

Detects `SELECT DISTINCT`.

Prefer explicit deduplication using keys and `row_number()` when deduplication is intentional.
