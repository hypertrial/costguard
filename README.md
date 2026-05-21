# costguard

A Rust-native pre-execution cost and performance analyzer for dbt and warehouse SQL.

## Why

Analytics engineers often discover expensive SQL only after CI or warehouse execution.
`costguard` catches common cost and performance risks locally before warehouse work runs.

## Install

```bash
cargo install --path crates/costguard-cli
```

## Usage

```bash
costguard scan models/ --warehouse snowflake
costguard scan models/ --warehouse bigquery --format json
costguard explain models/marts/fct_orders.sql
costguard pr --base main
costguard rules
```

## Configuration

`costguard.toml` is optional. CLI flags override file settings.

```toml
warehouse = "snowflake"
dialect = "snowflake"

[scan]
paths = ["models"]
ignore = ["target", "dbt_packages"]

[output]
format = "text"
fail_on = "high"

[dbt]
manifest_path = "target/manifest.json"

[rules.SQLCOST002]
threshold = 3
```

Exit codes:

- `0`: completed without diagnostics at or above the fail threshold
- `1`: diagnostics met or exceeded the fail threshold
- `2`: configuration error
- `3`: runtime error

## What It Detects

- full-scan incremental models
- repeated JSON extraction
- repeated regex work
- unbounded and cross joins
- `SELECT *` in downstream models
- blind `SELECT DISTINCT`
- unpartitioned window functions
- raw sources used directly in marts
- row-wise Python dbt model logic

## Non-goals

- not a dbt replacement
- not a SQL formatter
- not a SaaS monitor
- not a warehouse optimizer
- does not execute Jinja or connect to warehouses

## Status

Experimental.
