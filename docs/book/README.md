# Costguard

Costguard is a local, dbt-aware cost regression guardrail for git workflows.

One binary and one simple CI Action. `costguard pr` scans changed models against the git base. Runs locally as a fast Rust CLI with no warehouse credentials required.

Version `2.0.0` is the MVP production release for Generic SQL, Snowflake, BigQuery, and Trino. Databricks, Redshift, Postgres, and DuckDB remain preview. See [Platforms and warehouses](reference/platforms.md) and the [compatibility policy](reference/compatibility.md).

## Why

Analytics teams introduce warehouse cost and performance risks through normal PRs: unsafe incremental models,
repeated JSON parsing, unbounded joins, blind `SELECT DISTINCT`, missing partition predicates, expensive regex,
direct raw-source usage, and row-wise Python logic.

```text
PR opened -> changed SQL/dbt files scanned -> cost/perf risks annotated -> fail on high-risk findings
```

## Install

```bash
cargo install --path crates/costguard-cli
```

## Quick start

```bash
costguard pr --base origin/main --warehouse snowflake --fail-on high --min-confidence high
```

The CLI default for `--base` is `main`. In CI, prefer `origin/main` after `actions/checkout` with `fetch-depth: 0`.

See [Quick start (PR check)](getting-started/quick-start.md) for GitHub Action setup and [CLI reference](reference/cli.md) for all commands.

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

`v2.0.0` is the MVP production release. Startup standard mode and Git-native enterprise strict mode are technically complete; hosted control plane, SSO, commercial SLA, and procurement services are outside the product scope.
