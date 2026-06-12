# CLI reference

Source: `crates/costguard-cli/src/main.rs`

## Subcommands

| Command | Purpose |
| --- | --- |
| `scan` | Scan one or more paths (default: entire cwd) |
| `explain` | Analyze a single SQL/dbt file |
| `pr` | Scan git-changed files against a base ref |
| `cost` | Project cost report (model-centric totals) |
| `rules` | List registered rules |

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

## `scan`

```bash
costguard scan [PATHS...] [OPTIONS]
```

| Flag | Default | Notes |
| --- | --- | --- |
| `--fail-on` | `high` (via config default) | Severities: `info`, `low`, `medium`, `med`, `high`, `critical`, `crit` |
| `--min-confidence` | unset | Optional floor: `low`, `medium`/`med`, `high`. Recommended for noisy repos: `--fail-on high --min-confidence high` |
| `--baseline` | unset | Suppress findings matching baseline fingerprints |
| `--write-baseline` | unset | Snapshot findings to a baseline JSON file |
| `--cost` | unset | Enable per-finding savings estimates and project cost summary |
| `--fail-on-cost-delta` | unset | Optional deduplicated savings p50 gate (USD) on new findings |

## `explain`

```bash
costguard explain PATH [OPTIONS]
```

Requires exactly one file path. Supports `--cost`. Does not support `--fail-on`.

## `cost`

```bash
costguard cost [PATHS...] [OPTIONS]
```

Renders a project cost report (model totals, top models, optional finding savings). Cost is always enabled; uses `[cost]` from `costguard.toml` when present.

## `pr`

```bash
costguard pr [OPTIONS]
```

| Flag | Default | Notes |
| --- | --- | --- |
| `--base` | `main` | Git ref for changed-file detection; CI examples use `origin/main` |
| `--fail-on` | `high` | Same severity values as `scan` |
| `--min-confidence` | unset | Same confidence values as `scan` |
| `--baseline` | unset | Suppress findings matching baseline fingerprints |
| `--cost` | unset | Enable per-finding savings estimates and project cost summary |
| `--fail-on-cost-delta` | unset | Optional deduplicated savings p50 gate (USD) on new findings |

Invalid git bases and non-git directories fail the check instead of silently scanning zero files.

## `rules`

```bash
costguard rules [--format text|json|markdown|sarif]
```

## Manifest discovery

When `--manifest` is not passed:

1. Use `target/manifest.json` under the scan root if it exists
2. Otherwise use a discovered `manifest.json` in the project tree

Explicit `--manifest` paths must exist or the run exits with code `2`.

## Related

- [Configuration](configuration.md)
- [Platforms](platforms.md)
- [Output formats](output.md)
