# Platforms and warehouses

Costguard uses a single `Platform` enum for warehouse SQL dialect selection. CLI `--warehouse` and `--dialect` both set this value; `costguard.toml` accepts `warehouse` (preferred) or `dialect`.

## Supported values

| CLI / config value | Aliases | Support | Notes |
| --- | --- | --- | --- |
| `generic` | - | Production | Default; dialect-agnostic heuristics |
| `snowflake` | - | Production | Snowflake SQL |
| `bigquery` | - | Production | BigQuery SQL |
| `trino` | `presto` | Production | Presto/Trino SQL with compiled SQL normalization |
| `databricks` | - | Preview | Databricks SQL |
| `redshift` | - | Preview | Redshift SQL |
| `postgres` | `postgresql` | Preview | PostgreSQL SQL |
| `duckdb` | - | Preview | DuckDB SQL |

Production platforms have zero-parse-failure release fixtures and are covered by the v1 compatibility policy. Preview platforms have deterministic smoke coverage but may receive parser and rule behavior changes in minor releases.

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

Benchmark pins live in [`tests/benchmarks/repos.toml`](../../../tests/benchmarks/repos.toml).

## Related

- [CLI reference](cli.md)
- [Parse metrics](parse-metrics.md)
