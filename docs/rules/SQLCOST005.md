# SQLCOST005: Incremental model without date or partition predicate

Detects incremental models using `is_incremental()` without an obvious date or partition filter.

Add an `updated_at`, `event_date`, ingestion, or warehouse partition predicate to limit reads.
