# costguard

Costguard is a PR-first check for catching expensive dbt and warehouse SQL before it merges.

Runs locally as a fast Rust CLI. No warehouse credentials required.

## Quick start

```bash
cargo install --path crates/costguard-cli
costguard pr --base origin/main --warehouse snowflake --fail-on high
```

## Documentation

Full documentation is in the mdBook site under [`docs/book/`](docs/book/README.md).

Build and preview locally:

```bash
python3 scripts/generate_rule_docs.py
mdbook build
mdbook serve
```

| Topic | Link |
| --- | --- |
| PR check setup | [Quick start](docs/book/getting-started/quick-start.md) |
| CLI and config | [Reference](docs/book/reference/cli.md) |
| Rule catalog | [Rules](docs/book/rules/index.md) |
| Benchmarks | [Benchmark tiers](docs/book/contributing/benchmark-tiers.md) |
| Terminology | [Glossary](docs/book/glossary.md) |

## GitHub Action

```yaml
- uses: actions/checkout@v4
  with:
    fetch-depth: 0
- uses: ./.github/actions/costguard
  with:
    base: origin/main
    warehouse: snowflake
    fail-on: high
    format: github
```

See [Quick start (PR check)](docs/book/getting-started/quick-start.md) for inputs and dbt compile behavior.

## What it detects

Costguard ships 25 SQLCOST rules for incremental safety, join risk, warehouse cost patterns, and dbt anti-patterns. See the [rule catalog](docs/book/rules/index.md) for severity and fix guidance.

## Benchmark smoke tests

```bash
python3 scripts/benchmark_external_repo.py --all-vendored
python3 scripts/benchmark_external_repo.py --repo spellbook
cargo test -p costguard-core --test corpus
```

Layer definitions: [Benchmark tiers](docs/book/contributing/benchmark-tiers.md).

## Configuration sketch

```toml
warehouse = "snowflake"

[scan]
paths = ["models"]
ignore = ["target", "dbt_packages"]

[output]
fail_on = "high"

[dbt]
manifest_path = "target/manifest.json"
```

Full schema: [Configuration](docs/book/reference/configuration.md).

## Exit codes

| Code | Meaning |
| --- | --- |
| `0` | Pass |
| `1` | Findings at or above `--fail-on` |
| `2` | Config error |
| `3` | Runtime error |

## Status

Experimental.
