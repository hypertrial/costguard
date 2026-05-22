# SQLCOST018: UNION instead of UNION ALL

**Severity:** medium (high in staging/intermediate fan-in models)

Detects plain `UNION` in dbt models. `UNION` deduplicates and can force expensive global sorts or hash aggregation.

Prefer `UNION ALL` when duplicate rows are not expected.
