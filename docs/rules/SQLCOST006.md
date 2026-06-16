# SQLCOST006: Unbounded join risk

Detects joins without clear equality predicates.

Join on stable keys before applying transformations where possible. Function-wrapped join keys are reported separately by `SQLCOST017`.

Regex-only join detection (when SQL does not parse to AST) emits `confidence: low`. Use `--min-confidence medium` or `high` to suppress regex-only hits in PR gates.
