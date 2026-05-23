# SQLCOST017: Function-wrapped join key

**Severity:** high

Detects joins where either join key is transformed inline, such as `cast(a.id as varchar) = b.id`.

**Exemptions:** symmetric normalization on both sides (`lower(a.col) = lower(b.col)`), function-wrapped keys in staging models, time-bucket columns joined to `date_trunc(...)` on event time (for example `minute = date_trunc('minute', block_time)`), symmetric `date_trunc` / `coalesce` on both sides, and null-safe `coalesce(left.col, right.col) = dim.col` joins after full-outer merges.

Normalize keys once in staging, then join on stored normalized columns.
