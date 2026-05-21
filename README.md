# costguard

Costguard is a PR-first check for catching expensive dbt and warehouse SQL before it merges.

Runs locally as a fast Rust CLI. No warehouse credentials required.

## Why

Analytics teams usually introduce warehouse cost and performance risks through normal PRs:
unsafe incremental models, repeated JSON parsing, unbounded joins, blind `SELECT DISTINCT`,
missing partition predicates, expensive regex, direct raw-source usage, and row-wise Python logic.

Costguard is designed around this workflow:

```text
PR opened -> changed SQL/dbt files scanned -> cost/perf risks annotated -> fail on high-risk findings -> merge safer analytics code
```

Think of the CLI as the engine. The primary product workflow is automated PR review.

```text
Costguard CLI = engine
Costguard GitHub Action = primary product workflow
Costguard pre-commit = secondary workflow
Costguard local scan = developer/debug workflow
```

The current MVP ships the CLI engine first; a GitHub Action should be the primary packaged workflow.

## Install

```bash
cargo install --path crates/costguard-cli
```

## Usage

```bash
costguard pr --base origin/main --warehouse snowflake --fail-on high
costguard scan models/ --warehouse snowflake
costguard scan models/ --warehouse bigquery --format json
costguard explain models/marts/fct_orders.sql
costguard rules
```

The MVP fails builds by risk severity, not estimated dollars:

```bash
costguard pr --base origin/main --warehouse snowflake --fail-on high
```

Dollar-cost estimates can be added later as enrichment once warehouse metadata, history,
pricing, and execution-plan signals are available.

## Real-world Stress Testing

The first planned public-real stress target is Dune Spellbook. See
[`docs/design/spellbook-stress-test.md`](docs/design/spellbook-stress-test.md)
for the command set, metrics, and benchmark tiers.

See [`docs/design/pr-check-primary-workflow.md`](docs/design/pr-check-primary-workflow.md)
for the product workflow priority.

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
