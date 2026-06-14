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
| `baseline` | Migrate or generate identity maps for baseline v3 |
| `policy` | Compile, sign, verify, resolve, inspect, or migrate signed policy |

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
| `--fail-on-cost-delta` | scan, pr | Fail when deduplicated savings p50 on new findings exceeds threshold (USD) |
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
| `--fail-on-cost-delta` | unset | Optional deduplicated savings p50 gate (USD) on new findings |

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
| `--fail-on-cost-delta` | unset | Optional deduplicated savings p50 gate (USD) on new findings |

Invalid git bases and non-git directories fail the check instead of silently scanning zero files. Unchanged parse failures and other project-wide issues appear in the optional `context` report only; they do not fail the PR gate.

## `baseline`

```bash
costguard baseline identity-map-v2 OUTPUT [PATHS...] [OPTIONS]
costguard baseline migrate-v2 INPUT MAP OUTPUT [--policy BUNDLE --trust-store TRUST]
costguard baseline migrate-v1 INPUT OUTPUT [PATHS...] [OPTIONS]
```

| Command | Purpose |
| --- | --- |
| `identity-map-v2` | Scan the project and write an ordinal→semantic identity map (required before `migrate-v2` or `policy migrate-v1`) |
| `migrate-v2` | Convert baseline v2 to baseline v3 using the identity map |
| `migrate-v1` | Convert internal pre-v2 legacy baselines to baseline v3 (re-scans and matches by rule/path/message) |

`identity-map-v2` accepts the same `--warehouse`, `--manifest`, and signed-policy flags as scan. `migrate-v2` optionally binds `policy_digest` when both `--policy` and `--trust-store` are supplied.

Migration failures (missing map entries, platform mismatch, invalid input) exit with code `2`.

## `policy`

```bash
costguard policy keygen KEY_ID --private-key PATH --trust-store PATH
costguard policy compile INPUT.toml OUTPUT.json
costguard policy sign POLICY.json KEY.private.json OUTPUT.signed.json
costguard policy verify BUNDLE.json TRUST.json
costguard policy resolve BUNDLE.json TRUST.json --organization ORG --repository OWNER/REPO [--team TEAM] [--path PATH]
costguard policy inspect BUNDLE.json TRUST.json
costguard policy migrate-v1 INPUT MAP OUTPUT --version VERSION --issued-at RFC3339 [--expires-at RFC3339]
```

Signed policy bundles are distributed as pinned git artifacts. Compiled policy documents require `schema_version: 2` and `identity_scheme: "semantic-v1"`. See [Signed policy and exceptions](../governance/policy.md).

`policy migrate-v1` remaps v1 exception finding IDs through the identity map produced by `baseline identity-map-v2`. Sign the migrated JSON before deploying.

## `rules`

```bash
costguard rules [--format text|json|markdown|sarif]
```

## Exit codes

| Code | Meaning |
| --- | --- |
| `0` | Success: scan/pr passed gates; `explain` analysis complete; migration succeeded; `rules` listed |
| `1` | scan/pr: analysis incomplete, findings at/above `--fail-on` (with optional `--min-confidence`), or cost gate exceeded; `explain`: analysis incomplete only |
| `2` | Configuration error (invalid config, missing manifest, unsupported baseline v2 or policy v1, migration failure) |
| `3` | Runtime error (unexpected failure) |

See [Output formats](output.md) for JSON field details.

## Manifest discovery

When `--manifest` is not passed:

1. Use `target/manifest.json` under the scan root if it exists
2. Otherwise use a discovered `manifest.json` in the project tree

Explicit `--manifest` paths must exist or the run exits with code `2`.

## Related

- [Configuration](configuration.md)
- [Compatibility policy](compatibility.md)
- [Platforms](platforms.md)
- [Output formats](output.md)
