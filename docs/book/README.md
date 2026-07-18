# Stop expensive data-model regressions before merge.

SlowQL finds SQL problems. Costguard governs dbt and Rocky cost changes.

Costguard reviews analytics pull requests before merge and gates only cost findings introduced or regressed by the change.

It scans changed models against the git base, uses optional dbt manifests and sealed Rocky compile artifacts for lineage, and runs without warehouse credentials or live queries.

One binary and one simple CI Action. `costguard pr` is the main workflow; `costguard scan` is the local debugging path.

Measured on real dbt benchmark repos and the corpus suite:

- **97.2%** overall sampled precision
- **99.8%** high-severity sampled precision
- **44/44** behavioral rules passing TP census

Costguard supports Generic SQL, Snowflake, BigQuery, and Trino in production. Databricks, Redshift, Postgres, and DuckDB remain preview. See [Platforms and warehouses](reference/platforms.md) and the [compatibility policy](reference/compatibility.md).

## Why

Analytics teams introduce warehouse cost and performance risks through normal PRs: unsafe incremental models,
repeated JSON parsing, unbounded joins, blind `SELECT DISTINCT`, missing partition predicates, expensive regex,
direct raw-source usage, and row-wise Python logic.

```text
PR opened -> changed SQL/dbt/Rocky models scanned -> cost/perf risks annotated -> fail on high-risk findings
```

## Costguard vs SlowQL

SlowQL is the broader SQL analyzer: security, compliance, reliability, quality, performance, cost, schema checks, autofix, custom rules, and editor integration.

Costguard is the dbt change-control layer: base/head finding and cost deltas, lineage and exposure impact, owners, gates, waivers, coverage, and versioned evidence. See [Costguard vs SlowQL](reference/costguard-vs-slowql.md).

## Install

```bash
cargo install --path crates/costguard-cli
```

## Quick start

Try it locally with no config file or flags:

```bash
cd your-dbt-project
costguard scan   # scans the whole project, auto-detects target/manifest.json
```

For CI, scan changed files against a git base:

```bash
costguard pr --base origin/main --warehouse snowflake --fail-on high --min-confidence high
```

Costguard can route findings to dbt/CODEOWNERS owners, apply scoped PR gates and expiring waivers, and write GitHub annotations, markdown, and a JSON v4 receipt from one scan.

The CLI default for `--base` is `main`. In CI, prefer `origin/main` after `actions/checkout` with `fetch-depth: 0`.

See [Local scan and explain](getting-started/local-scan.md) for the zero-config local workflow, [Rocky integration](getting-started/rocky.md) for sealed compile metadata, [Quick start (PR check)](getting-started/quick-start.md) for GitHub Action setup, and [CLI reference](reference/cli.md) for all commands.

## Documentation map

| Section | Contents |
| --- | --- |
| [Getting started](getting-started/quick-start.md) | PR check, local scan, pre-commit status |
| [Reference](reference/cli.md) | CLI, config, output, suppressions, parse metrics, platforms, scripts |
| [Rules](rules/index.md) | Rule catalog with severity and fix guidance |
| [Design](../design/pr-check-primary-workflow.md) | Product workflow, benchmarks, Spellbook stress test |
| [Contributing](contributing/benchmark-tiers.md) | Benchmark layers, corpus fixtures |
| [Glossary](glossary.md) | Canonical terminology |

## Status

Startup standard mode and Git-native enterprise strict mode are technically complete; hosted control plane, SSO, commercial SLA, and procurement services are outside the product scope.
