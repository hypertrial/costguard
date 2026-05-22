# Platforms and warehouses

Costguard uses a single `Platform` enum for warehouse SQL dialect selection. CLI `--warehouse` and `--dialect` both set this value; `costguard.toml` accepts `warehouse` (preferred) or `dialect`.

## Supported values

| CLI / config value | Aliases | Notes |
| --- | --- | --- |
| `generic` | — | Default; dialect-agnostic heuristics |
| `snowflake` | — | Snowflake SQL |
| `bigquery` | — | BigQuery SQL |
| `databricks` | — | Databricks SQL |
| `redshift` | — | Redshift SQL |
| `postgres` | `postgresql` | PostgreSQL SQL |
| `duckdb` | — | DuckDB SQL |
| `trino` | `presto` | Presto/Trino SQL (Hive-family parsing + Trino normalization for compiled SQL) |

## Choosing a platform

| Project type | Recommended value |
| --- | --- |
| Generic dbt tutorials | `generic` |
| Snowflake warehouses | `snowflake` |
| Dune Spellbook / Trino dbt | `trino` |
| jaffle-shop (Postgres adapter) | `generic` or `postgres` in benchmarks |

## Defaults by context

| Context | warehouse | fail-on |
| --- | --- | --- |
| PR examples / MVP docs | `snowflake` (illustrative) | `high` |
| External benchmark: spellbook | `trino` | `critical` |
| External benchmark: jaffle-shop | `generic` | `critical` |
| Vendored harness | `generic` | `critical` |
| This repo's PR workflow | `generic` | `high` |

Benchmark pins live in [`tests/benchmarks/repos.toml`](../../tests/benchmarks/repos.toml).

## Related

- [CLI reference](cli.md)
- [Parse metrics](parse-metrics.md)
