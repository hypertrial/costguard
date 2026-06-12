# Privacy and local-only guarantees

Costguard is a **local static analyzer**. It is suitable for enterprise security review when these properties matter:

## What Costguard does not do

- Does **not** connect to Snowflake, BigQuery, Trino, or any warehouse.
- Does **not** execute SQL against production or development databases.
- Does **not** read warehouse credentials (dbt compile uses dummy `profiles.yml` in CI helpers).
- Does **not** send telemetry, crash reports, or usage analytics to Hypertrial or third parties.
- Does **not** upload repository contents during scan (GitHub Action downloads the Costguard binary only).

## What Costguard reads locally

- SQL, YAML, Python, and manifest files under configured scan paths.
- Optional git metadata for `costguard pr` changed-file detection.
- User-supplied `costguard.toml`, finding baselines, and suppressions in source files.
- Optional local cost inputs: dbt `catalog.json`, offline query-history CSV exports, and `[cost]` overrides in `costguard.toml`. Costguard reads these files only; it does not fetch them from a warehouse.

## What leaves the runner

Only what **you configure** in CI output:

- `github` / `markdown` annotations in the job log.
- `json` or `sarif` artifacts you archive or upload.
- GitHub Action step summary when `format: markdown` is selected.

## Release integrity

- GitHub Releases ship with `SHA256SUMS` and optional provenance signing (see [Release checklist](../contributing/releasing.md)).
- The composite Action verifies tarball checksums before install.

## Questions

Report security concerns via [SECURITY.md](../../../SECURITY.md).
