# SQLCOST023: Scan without dbt manifest

**Severity:** info

Reports when Costguard scans dbt SQL and schema YAML without a compiled `target/manifest.json`.

Run `dbt compile` and pass `--manifest target/manifest.json` for richer model metadata and compiled SQL parse metrics. Or run `dbt compile && costguard scan` from the project root.
