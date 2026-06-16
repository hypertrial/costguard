# SQLCOST045: Stale dbt manifest

**Severity:** info

Reports when `target/manifest.json` is older than one or more modified dbt model SQL files. Costguard may analyze stale `compiled_code` from the manifest.

## When it fires

- A manifest is loaded (auto-detected or via `--manifest`).
- At least one scanned dbt model SQL file has a modification time strictly newer than the manifest file.

## Fix

Re-run dbt compile so the manifest reflects current models, then scan again:

```bash
dbt compile && costguard scan
```

Under `--analysis-policy strict`, a stale manifest fails the scan (same as a missing manifest).

## Note

Staleness uses file modification times. In a fresh git checkout, mtimes may not reflect edit history; run `dbt compile` in CI before Costguard for a reliable manifest.
