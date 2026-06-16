# Requirements

Costguard reads **source files, git history, and (optionally) `target/manifest.json`**. It never connects to your warehouse and never needs credentials.

## What you need

| Input | Required? | Notes |
| --- | --- | --- |
| Git repository | Yes for `costguard pr` | PR mode diffs against a base ref; local `scan` works without git history |
| Full git history (`fetch-depth: 0`) | Yes for CI PR checks | Shallow checkout breaks `--base origin/main` changed-file detection |
| SQL / dbt model files | Yes | Scanned from the project root (or `[scan].paths`) |
| `target/manifest.json` | Optional | Auto-detected when present; use compiled SQL for Jinja-heavy models |
| `dbt compile` | Optional (your CI step) | Costguard does not run dbt; run compile before the check when you want manifest-backed analysis |
| `--warehouse` / dialect | Optional | Default `generic`; set to your warehouse for sharper parsing |
| Warehouse credentials | Never | Local-only analysis |
| Python 3 on CI runner | Yes for GitHub Action | The composite action driver is Python |
| Cost inputs (`catalog.json`, query history, observations) | Optional | Only when `--cost` / `[cost]` is enabled |

## Manifest behavior

If `--manifest` is omitted, Costguard auto-loads `target/manifest.json` when that file exists in the scan root. Raw source analysis still works without a manifest; compiled-SQL metrics and some Jinja-heavy findings improve when a manifest is present.

For Jinja-heavy dbt models, run `dbt compile` in your existing CI job before Costguard so `compiled_code` is available in the manifest.

## Related

- [Installation](installation.md)
- [Local scan and explain](local-scan.md)
- [Quick start (PR check)](quick-start.md)
- [Configuration](../reference/configuration.md)
