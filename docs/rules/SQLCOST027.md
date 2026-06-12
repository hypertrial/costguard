# SQLCOST027 — SQL parse failure

Reports when a dbt model SQL file could not be parsed. Rules may fall back to regex heuristics with lower confidence.

## Severity

Info

## When it fires

- A model-scoped SQL file fails both raw and compiled parse (when manifest is available).

## Suggestion

Run `dbt compile` and pass `--manifest target/manifest.json` for compiled SQL fallback.

## Suppression

Not typically suppressed; fix parse issues or accept regex fallback with `--min-confidence high` on CI gates.
