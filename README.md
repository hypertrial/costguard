# costguard

Costguard is a PR-first check for catching expensive dbt and warehouse SQL before it merges.

Runs locally as a fast Rust CLI. No warehouse credentials required.

Costguard 1.0 is production-ready for Generic SQL, Snowflake, BigQuery, and Trino. Databricks, Redshift, Postgres, and DuckDB support is preview.

## Quick start

Use the GitHub Action at the moving compatible major tag `@v1`, pin exact behavior with `@v1.0.0`, or download a checksum-protected binary from GitHub Releases.

```yaml
- uses: actions/checkout@v4
  with:
    fetch-depth: 0
- uses: hypertrial/costguard/.github/actions/costguard@v1
  with:
    base: origin/main
    warehouse: snowflake
    fail-on: high
    min-confidence: high
```

Install an exact release binary by selecting one of `aarch64-apple-darwin`, `x86_64-apple-darwin`, or `x86_64-unknown-linux-gnu`:

```bash
VERSION=v1.0.0
TARGET=aarch64-apple-darwin
curl -LO "https://github.com/hypertrial/costguard/releases/download/${VERSION}/costguard-${TARGET}.tar.gz"
curl -LO "https://github.com/hypertrial/costguard/releases/download/${VERSION}/costguard-${TARGET}.tar.gz.sha256"
shasum -a 256 -c "costguard-${TARGET}.tar.gz.sha256"
tar -xzf "costguard-${TARGET}.tar.gz"
./costguard --version
```

Windows x86-64 uses `costguard-x86_64-pc-windows-msvc.tar.gz` and contains `costguard.exe`. Every release also includes consolidated `SHA256SUMS`, native smoke receipts, and signed provenance.

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
- uses: hypertrial/costguard/.github/actions/costguard@v1
  with:
    base: origin/main
    warehouse: snowflake
    fail-on: high
    min-confidence: high
    format: github
```

When developing Costguard itself, use the same action with `install-mode: source` so the workflow builds the checked-out code instead of downloading a release.

When dbt compilation is enabled, the Action derives the dbt adapter from `warehouse`. For `warehouse: generic`, set `dbt-adapter-package` explicitly.

Recommended for macro-heavy dbt repos: pair `--fail-on high` with `--min-confidence high` to suppress regex-only findings while keeping AST-confirmed high-risk hits.

See [Quick start (PR check)](docs/book/getting-started/quick-start.md) for inputs and dbt compile behavior.

## What it detects

Costguard ships 26 SQLCOST rules for incremental safety, join risk, warehouse cost patterns, and dbt anti-patterns. See the [rule catalog](docs/book/rules/index.md) for severity and fix guidance.

## Benchmark smoke tests

```bash
python3 scripts/benchmark_external_repo.py --all-vendored
python3 scripts/benchmark_external_repo.py --repo spellbook --smoke
python3 scripts/benchmark_external_repo.py --repo spellbook   # full gate (manual / baseline refresh)
cargo test -p costguard-core --test corpus
```

Layer definitions: [Benchmark tiers](docs/book/contributing/benchmark-tiers.md).

While GitHub Actions CI is unavailable, run the full local gate with `./scripts/ci_local.sh`.

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
| `1` | Findings at or above `--fail-on` with confidence at or above `--min-confidence` when set |
| `2` | Config error |
| `3` | Runtime error |

## Status

Production-ready. See the [support policy](SUPPORT.md), [compatibility policy](docs/book/reference/compatibility.md), and [security policy](SECURITY.md).
