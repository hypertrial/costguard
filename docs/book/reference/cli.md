# CLI reference

Source: `crates/costguard-cli/src/main.rs`

## Subcommands

| Command | Purpose |
| --- | --- |
| `scan` | Scan one or more paths (default: entire cwd) |
| `explain` | Analyze a single SQL/dbt file |
| `pr` | Scan git-changed files against a base ref |
| `cost` | Local cost prioritization summary (model-centric totals for ranking) |
| `rules` | List registered rules |
| `init` | Scaffold GitHub workflow and starter `costguard.toml` into a dbt project |
| `policy` | Compile, sign, verify, resolve, or inspect signed policy |

## Shared flags

| Flag | Subcommands | Description |
| --- | --- | --- |
| `--warehouse` | scan, explain, pr | Platform/dialect for SQL parsing heuristics |
| `--dialect` | scan, explain, pr | Alias for `--warehouse` (config file uses either key) |
| `--format` | scan, explain, pr, rules | `text`, `json`, `github`, `markdown`, `sarif` (config also accepts `md`) |
| `--manifest` | scan, explain, pr | Path to dbt `manifest.json` with `compiled_code` |
| `--baseline` | scan, pr | Finding baseline JSON (grandfather known findings) |
| `--write-baseline` | scan | Write current findings to a baseline JSON file |
| `--cost` | scan, explain, pr | Enable cost estimates (uses `[cost]` in `costguard.toml` when present) |
| `--fail-on-cost-delta` | scan, pr | Fail when **addressable finding savings** p50 on new findings exceeds threshold (USD) |
| `--analysis-policy` | scan, explain, pr, cost | `standard` or `strict`; see `[analysis]` in [Configuration](configuration.md) |
| `--policy` | scan, explain, pr, cost | Signed policy bundle JSON path; see `[policy]` in [Configuration](configuration.md) |
| `--trust-store` | scan, explain, pr, cost | Public trust store for `--policy` verification |
| `--policy-organization` | scan, explain, pr, cost | Organization slug for signed policy resolution |
| `--policy-team` | scan, explain, pr, cost | Optional team slug for signed policy resolution |
| `--policy-repository` | scan, explain, pr, cost | Repository slug (`owner/repo`) for signed policy resolution |

## `scan`

```bash
costguard scan [PATHS...] [OPTIONS]
```

| Flag | Default | Notes |
| --- | --- | --- |
| `--fail-on` | `high` (via config default) | Severities: `info`, `low`, `medium`, `med`, `high`, `critical`, `crit` |
| `--min-confidence` | unset | Optional floor: `low`, `medium`/`med`, `high`. Recommended for noisy repos: `--fail-on high --min-confidence high` |
| `--baseline` | unset | Suppress findings matching baseline fingerprints (requires baseline v3) |
| `--write-baseline` | unset | Snapshot findings to a baseline v3 JSON file |
| `--cost` | unset | Enable per-finding savings estimates and cost prioritization summary |
| `--fail-on-cost-delta` | unset | Optional **addressable finding savings** p50 gate (USD) on new findings |

## `explain`

```bash
costguard explain PATH [OPTIONS]
```

Requires exactly one file path. Supports `--cost`. Does not support `--fail-on`, `--min-confidence`, or `--fail-on-cost-delta`.

**Exit behavior:** `explain` ignores severity and cost gates. It exits `0` when analysis completeness checks pass, even if high-severity findings are present. It exits `1` only when `analysis.passed` is false (for example strict-mode violations or expired exceptions).

## `cost`

```bash
costguard cost report [PATHS...] [OPTIONS]
costguard cost normalize INPUT OUTPUT --source snowflake --organization ORG --repository OWNER/REPO --provenance EXPORT_ID
```

Renders a local cost prioritization summary (model totals, top models, optional finding savings). Cost is always enabled; uses `[cost]` from `costguard.toml` when present.

## `pr`

```bash
costguard pr [OPTIONS]
```

| Flag | Default | Notes |
| --- | --- | --- |
| `--base` | `main` | Git ref for changed-file detection; CI examples use `origin/main` |
| `--fail-on` | `high` | Same severity values as `scan` |
| `--min-confidence` | unset | Same confidence values as `scan` |
| `--baseline` | unset | Suppress findings matching baseline fingerprints (requires baseline v3) |
| `--cost` | unset | Enable per-finding savings estimates and cost prioritization summary |
| `--fail-on-cost-delta` | unset | Optional **addressable finding savings** p50 gate (USD) on new findings |

Invalid git bases and non-git directories fail the check instead of silently scanning zero files. Unchanged parse failures and other project-wide issues appear in the optional `context` report only; they do not fail the PR gate.

Finding baselines are written with `costguard scan --write-baseline` (baseline v3 with `identity_scheme: "semantic-v1"`).

## `init`

```bash
costguard init [--warehouse PLATFORM] [--force] [--no-workflow] [--no-config]
```

Scaffolds Costguard into the current directory (typically a dbt project root):

- `.github/workflows/costguard.yml` — PR check using the published GitHub Action
- `costguard.toml` — starter config with detected or overridden `warehouse`

Warehouse detection is best-effort: reads `dbt_project.yml` `profile`, then `profiles.yml` (project root or `~/.dbt/profiles.yml`) adapter `type`. Use `--warehouse` when detection fails. Existing files are skipped unless `--force`.

## `policy`

```bash
costguard policy keygen KEY_ID --private-key PATH --trust-store PATH
costguard policy compile INPUT.toml OUTPUT.json
costguard policy sign POLICY.json KEY.private.json OUTPUT.signed.json
costguard policy verify BUNDLE.json TRUST.json
costguard policy resolve BUNDLE.json TRUST.json --organization ORG --repository OWNER/REPO [--team TEAM] [--path PATH]
costguard policy inspect BUNDLE.json TRUST.json
```

Signed policy bundles are distributed as pinned git artifacts. Compiled policy documents require `schema_version: 2` and `identity_scheme: "semantic-v1"`. See [Signed policy and exceptions](../governance/policy.md).

## `rules`

```bash
costguard rules [--format text|json|markdown|sarif]
```

## Exit codes

| Code | Meaning |
| --- | --- |
| `0` | Success: scan/pr passed gates; `explain` analysis complete; `rules` listed |
| `1` | scan/pr: analysis incomplete, findings at/above `--fail-on` (with optional `--min-confidence`), or cost gate exceeded; `explain`: analysis incomplete only |
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
