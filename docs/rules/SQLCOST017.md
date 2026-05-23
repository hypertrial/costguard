# SQLCOST017: Function-wrapped join key

**Severity:** high

Detects joins where either join key is transformed inline, such as `cast(a.id as varchar) = b.id`.

**Exemptions:** symmetric normalization on both sides (`lower(a.col) = lower(b.col)`), and function-wrapped keys in staging models.

Normalize keys once in staging, then join on stored normalized columns.
