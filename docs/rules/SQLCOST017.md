# SQLCOST017: Function-wrapped join key

**Severity:** high

Detects joins where either join key is transformed inline, such as `lower(a.email) = lower(b.email)` or `cast(a.id as varchar) = b.id`.

Normalize keys once in staging, then join on stored normalized columns.
