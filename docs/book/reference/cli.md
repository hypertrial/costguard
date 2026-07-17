# CLI reference

Source: thin entry point in `crates/costguard-cli/src/main.rs`, Clap root in `args.rs`, and command dispatch/handlers in `commands.rs`.

## Subcommands

| Command | Purpose |
| --- | --- |
| `scan` | Scan one or more paths (default: entire cwd) |
| `explain` | Analyze a single SQL/dbt file |
| `pr` | Scan git-changed files against a base ref |
| `cost` | Local cost prioritization summary (model-centric totals for ranking) |
| `rules` | List registered rules |
| `init` | Scaffold GitHub workflow and starter `costguard.toml` into a dbt project |
| `doctor` | Run read-only project and PR-workflow readiness checks |
| `policy` | Compile, sign, verify, resolve, or inspect signed policy |

## Shared flags

| Flag | Subcommands | Description |
| --- | --- | --- |
| `--warehouse` | scan, explain, pr | Platform/dialect for SQL parsing heuristics |
| `--dialect` | scan, explain, pr | Alias for `--warehouse` (config file uses either key) |
| `--format` | scan, explain, pr, rules | `text`, `json`, `github`, `markdown`, `sarif` (config also accepts `md`) |
| `--manifest` | scan, explain, pr | Path to dbt `manifest.json` with `compiled_code` |
| `--base-manifest` | scan, explain, pr | Optional production/state manifest for PR base-vs-head comparison |
| `--baseline` | scan, pr | Finding baseline JSON (grandfather known findings) |
| `--write-baseline` | scan | Write current findings to a baseline JSON file |
| `--cost` | scan, explain, pr | Enable cost estimates (uses `[cost]` in `costguard.toml` when present) |
| `--fail-on-cost-delta` | scan, pr | Fail when **addressable finding savings** p50 on new findings exceeds threshold (USD) |
| `--min-cost-coverage` | scan, pr | Fail when mapped-spend coverage is below fraction (0.0–1.0); implies `--cost` |
| `--analysis-policy` | scan, explain, pr, cost | `standard` or `strict`; see `[analysis]` in [Configuration](configuration.md) |
| `--policy` | scan, explain, pr, cost | Signed policy bundle JSON path; see `[policy]` in [Configuration](configuration.md) |
| `--trust-store` | scan, explain, pr, cost | Public trust store for `--policy` verification |
| `--policy-organization` | scan, explain, pr, cost | Organization slug for signed policy resolution |
| `--policy-team` | scan, explain, pr, cost | Optional team slug for signed policy resolution |
| `--policy-repository` | scan, explain, pr, cost | Repository slug (`owner/repo`) for signed policy resolution |
| `--summary-file` | scan, pr | Write markdown from the same scan while stdout keeps `--format` |
| `--receipt-file` | scan, pr | Write the full JSON v4 receipt from the same scan |
| `--compare-receipt` | scan, pr | Compare with a prior JSON v4 receipt and add `pr_summary.trend` |

## `scan`

```bash
costguard scan [PATHS...] [OPTIONS]
```

| Flag | Default | Notes |
| --- | --- | --- |
| `--fail-on` | `high` (via config default) | Severities: `info`, `low`, `medium`, `med`, `high`, `critical`, `crit` |
| `--min-confidence` | unset | Optional floor: `low`, `medium`/`med`, `high`. Recommended for noisy repos: `--fail-on high --min-confidence high` |
| `--min-confidence-filter` | off | When set with `--min-confidence`, omit findings below the floor from output |
| `--baseline` | unset | Suppress findings matching baseline fingerprints (requires baseline v3) |
| `--write-baseline` | unset | Snapshot findings to a baseline v3 JSON file |
| `--cost` | unset | Enable per-finding savings estimates and cost prioritization summary |
| `--fail-on-cost-delta` | unset | Optional **addressable finding savings** p50 gate (USD) on new findings |
| `--min-cost-coverage` | unset | Optional mapped-spend coverage floor (0.0–1.0); implies `--cost` |
| `--summary-file` / `--receipt-file` | unset | Optional markdown summary and JSON v4 receipt files |
| `--compare-receipt` | unset | Prior JSON v4 receipt used for trend deltas |

## `explain`

```bash
costguard explain PATH [OPTIONS]
```

Requires exactly one file path. Supports `--cost`. Does not support `--fail-on`, `--min-confidence`, or `--fail-on-cost-delta`.

**Exit behavior:** `explain` ignores severity and cost gates. It exits `0` when analysis completeness checks pass, even if high-severity findings are present. It exits `1` only when `analysis.passed` is false (for example strict-mode violations or expired exceptions).

## `cost`

```bash
costguard cost report [PATHS...] [OPTIONS]
costguard cost normalize INPUT OUTPUT --source snowflake|bigquery|databricks|trino|pipeline|generic --organization ORG --repository OWNER/REPO --provenance EXPORT_ID
```

Renders a local cost prioritization summary (model totals, top models, optional finding savings). Cost is always enabled; uses `[cost]` from `costguard.toml` when present.

Use `--source pipeline` for local orchestration exports from tools such as dlt, Dagster, DuckDB, and dbt. It accepts common columns including `model_id`, `relation`, `relation_name`, `asset_key`, `dbt_model`, `window_start`, `window_end`, `executions`, `run_count`, `duration_seconds`, `duration_ms`, `bytes_processed`, and `cost_usd`; row-count columns are ignored in v2.6.0.

## `pr`

```bash
costguard pr [OPTIONS]
```

| Flag | Default | Notes |
| --- | --- | --- |
| `--base` | `main` | Git ref for changed-file detection; CI examples use `origin/main` |
| `--fail-on` | `high` | Same severity values as `scan` |
| `--min-confidence` | unset | Same confidence values as `scan` |
| `--min-confidence-filter` | off | Same as `scan`; omit findings below `--min-confidence` from output |
| `--baseline` | unset | Suppress findings matching baseline fingerprints (requires baseline v3) |
| `--cost` | unset | Enable per-finding savings estimates and cost prioritization summary |
| `--fail-on-cost-delta` | unset | Optional **addressable finding savings** p50 gate (USD) on new findings |
| `--block-only-new[=true\|false]` | unset (`false` config behavior) | PR-only override. Bare flag means `true`; filters severity and addressable-savings enforcement to introduced/regressed findings. |
| `--fail-on-pr-cost-increase` | unset | PR-only project net-cost gate in USD/month; requires priced `[cost]` configuration and implies cost analysis. |
| `--min-cost-coverage` | unset | Optional mapped-spend coverage floor (0.0–1.0); implies `--cost` |
| `--summary-file` / `--receipt-file` | unset | Optional markdown summary and JSON v4 receipt files |
| `--compare-receipt` | unset | Prior JSON v4 receipt used for trend deltas |

Invalid git bases and non-git directories fail the check instead of silently scanning zero files. With `--block-only-new`, introduced and severity/cost-regressed findings block; unchanged findings remain in evidence as nonblocking diagnostics, resolved findings never block, and a behavioral diagnostic without a semantic ID fails closed. Required-owner and blast-radius controls still evaluate every changed model.

`--fail-on-cost-delta` keeps its addressable finding-savings meaning. `--fail-on-pr-cost-increase` is distinct: it fails when the whole-PR priced `pr_impact.net.monthly_p50` is greater than or equal to the threshold. Missing pricing is configuration exit `2`; missing computed PR impact is an auditable failing gate result.

Unchanged parse failures and other project-wide issues appear in the optional `context` report only; they do not fail the PR gate.

Finding baselines are written with `costguard scan --write-baseline` (baseline v3 with `identity_scheme: "semantic-v1"`).

## `init`

```bash
costguard init [--warehouse PLATFORM] [--profile local-duckdb] [--dbt-dir PATH] [--force] [--no-workflow] [--no-config]
```

Scaffolds Costguard into the current directory (typically a dbt project root):

- `.github/workflows/costguard.yml` — PR check using the published GitHub Action
- `costguard.toml` — starter config with detected or overridden `warehouse` and `[gate] block_only_new = true`

The generated workflow also sets `block-only-new: true`, `pr-comment: true`, and least-privilege `pull-requests: write` explicitly. Warehouse detection is best-effort: reads `dbt_project.yml` `profile`, then `profiles.yml` (project root or `~/.dbt/profiles.yml`) adapter `type`. Use `--warehouse` when detection fails. Existing files are skipped unless `--force`.

`--profile local-duckdb` writes active DuckDB/dbt defaults, keeps strict analysis commented as an opt-in hardening step, and rejects non-DuckDB warehouse overrides. Use `--dbt-dir dbt` from a repository root when the dbt project lives in a subdirectory.

After scaffolding, `init` runs the same read-only report as `doctor`. Readiness warnings and blockers are printed for remediation, but a successful initialization still exits `0`.

## `doctor`

```bash
costguard doctor [--dbt-dir PATH]
```

Checks git history, analyzable files, configured policy, dbt metadata freshness/integrity, parse coverage, warehouse selection, the GitHub workflow contract, and mapped-spend/USD cost coverage. It performs a normal local scan but never runs dbt, connects to a warehouse, or writes files. Workflow validation parses YAML and requires one complete Costguard job with full-history checkout, Action inputs, and effective job/top-level permissions; comments and unrelated jobs do not count. With `--dbt-dir`, project config and models are loaded below that directory while the workflow is checked at the repository root.

## `policy`

```bash
costguard policy keygen KEY_ID --private-key PATH --trust-store PATH [--force]
costguard policy compile INPUT.toml OUTPUT.json
costguard policy sign POLICY.json KEY.private.json OUTPUT.signed.json
costguard policy verify BUNDLE.json TRUST.json
costguard policy resolve BUNDLE.json TRUST.json --organization ORG --repository OWNER/REPO [--team TEAM] [--path PATH]
costguard policy inspect BUNDLE.json TRUST.json
```

Signed policy bundles are distributed as pinned git artifacts. Compiled policy documents require `schema_version: 2` and `identity_scheme: "semantic-v1"`. See [Signed policy and exceptions](../governance/policy.md).

`policy keygen` refuses either existing output unless `--force` is supplied. Forced replacement accepts regular files only; identical paths, symlinks, directories, and special files are rejected. Private keys are atomically persisted with mode `0600` on Unix.

## `rules`

```bash
costguard rules [--format text|json|markdown|sarif]
```

## Exit codes

| Code | Meaning |
| --- | --- |
| `0` | Success: scan/pr passed gates; `explain` analysis complete; `rules` listed; `doctor` found no blockers |
| `1` | scan/pr: analysis incomplete, a blocking `[gate]` failed, findings met `--fail-on` (with optional `--min-confidence`), or a cost gate exceeded; `explain`: analysis incomplete; `doctor`: readiness blocker |
| `2` | Configuration error (invalid config, missing manifest, unsupported baseline or policy schema) |
| `3` | Runtime error (unexpected failure) |

See [Output formats](output.md) for JSON field details.

## Manifest discovery

When `--manifest` is not passed, Costguard auto-loads `target/manifest.json` when that file exists in the scan root. See [Requirements](../getting-started/requirements.md) for when a manifest is needed.

Explicit `--manifest` paths must exist or the run exits with code `2`.

## Related

- [Configuration](configuration.md)
- [Compatibility policy](compatibility.md)
- [Platforms](platforms.md)
- [Output formats](output.md)
