# Requirements

Costguard reads **source files, git history, and optional dbt/Rocky compile metadata**. It never connects to your warehouse and never needs credentials.

## What you need

| Input | Required? | Notes |
| --- | --- | --- |
| Git repository | Yes for `costguard pr` | PR mode diffs against a base ref; local `scan` works without git history |
| Full git history (`fetch-depth: 0`) | Yes for CI PR checks | Shallow checkout breaks `--base origin/main` changed-file detection |
| SQL / dbt model files | Yes | Scanned from the project root (or `[scan].paths`) |
| `target/manifest.json` | Optional | Auto-detected when present; use compiled SQL for Jinja-heavy models; default maximum 512 MiB |
| `dbt compile` | Optional (your CI step) | Costguard does not run dbt; run compile before the check when you want manifest-backed analysis |
| Sealed Rocky artifact | Required for `.rocky` DSL analysis | Run Rocky compile with expanded macros, then `costguard rocky capture`; Costguard never invokes Rocky |
| `--warehouse` / dialect | Optional | Default `generic`; set to your warehouse for sharper parsing |
| Warehouse credentials | Never | Local-only analysis |
| Python 3 on CI runner | Yes for GitHub Action | The composite action driver is Python |
| Cost inputs (`catalog.json`, query history, observations) | Optional | Only when `--cost` / `[cost]` is enabled |

## Manifest behavior

If `--manifest` is omitted, Costguard auto-loads `target/manifest.json` when that file exists in the scan root. Raw source analysis still works without a manifest; compiled-SQL metrics and some Jinja-heavy findings improve when a manifest is present.

When a manifest is loaded but model SQL files are newer than the manifest file, Costguard emits **SQLCOST045** (stale manifest). When changed models carry manifest sha256 checksums that do not match the workspace file, Costguard emits **SQLCOST046** (checksum mismatch). Under `--analysis-policy strict`, a stale manifest fails the scan until you re-run `dbt compile`.

For Jinja-heavy dbt models, run `dbt compile` in your existing CI job before Costguard so `compiled_code` is available in the manifest.

All explicit, auto-detected, and git-base manifests are limited by `[dbt].max_manifest_bytes` (default `536870912`). Omitted or `0` uses that default. Repositories with a legitimate larger manifest must raise the setting explicitly; oversized inputs fail closed before scan output is produced.

Rocky artifact behavior and exact-base requirements are documented in [Rocky integration](rocky.md).

Maintainer evaluation and judge environments require Python 3.11 or newer. Install their universal hashed locks with standard pip:

```bash
pip install --require-hashes -r requirements-eval.lock
pip install --require-hashes -r requirements-judge.lock
```

## Related

- [Installation](installation.md)
- [Local scan and explain](local-scan.md)
- [Quick start (PR check)](quick-start.md)
- [Configuration](../reference/configuration.md)
