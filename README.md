# costguard

Costguard is a local, dbt-aware cost regression guardrail for git workflows.

One binary and one simple CI Action. `costguard pr` scans changed models against the git base. Runs locally as a fast Rust CLI with no warehouse credentials required.

Generic SQL, Snowflake, BigQuery, and Trino scanning are production-ready; Databricks, Redshift, Postgres, and DuckDB support is preview.

## Quick start

Pin the exact Action tag `@v2.0.0` or use the moving compatible major tag `@v2`. Release binaries are checksum-protected and include provenance attestations. Until the `v2.0.0` GitHub release finishes publishing, pin `@main` and set `install-mode: source` (see [GitHub Action](#github-action)).

Run Costguard after your existing dbt compile step so `target/manifest.json` is available when you want manifest-backed analysis.

```yaml
- uses: actions/checkout@v6
  with:
    fetch-depth: 0
- run: dbt compile --target dev
- uses: hypertrial/costguard/.github/actions/costguard@v2.0.0
  with:
    base: origin/main
    warehouse: snowflake
    fail-on: high
    min-confidence: high
```

Install an exact release binary by selecting one of `aarch64-apple-darwin`, `x86_64-apple-darwin`, or `x86_64-unknown-linux-gnu`:

```bash
VERSION=v2.0.0
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
- uses: actions/checkout@v6
  with:
    fetch-depth: 0
- run: dbt compile --target dev
- uses: hypertrial/costguard/.github/actions/costguard@v2.0.0
  with:
    base: origin/main
    warehouse: snowflake
    manifest: target/manifest.json
    fail-on: high
    min-confidence: high
    format: github
```

Use `install-mode: source` to build the checked-out Action code instead of downloading a release binary. This is required for Costguard development and works immediately before the first `v2.0.0` release artifacts are published:

```yaml
- uses: hypertrial/costguard/.github/actions/costguard@main
  with:
    install-mode: source
    base: origin/main
    warehouse: snowflake
    fail-on: high
    min-confidence: high
```

The Action does not install or compile dbt. It auto-detects `target/manifest.json` when present; raw analysis still works without a manifest.

Only high-confidence, high-severity findings fail the PR by default. Pair `fail-on: high` with `min-confidence: high` on macro-heavy dbt repos.

See [Quick start (PR check)](docs/book/getting-started/quick-start.md) for inputs and workflow guidance.

## What it detects

Costguard ships **44 SQLCOST rules** for incremental safety, join risk, warehouse cost patterns, and dbt anti-patterns. Optional **[cost estimates](docs/book/reference/cost-estimates.md)** rank findings for prioritization using local catalog stats and offline exports—they are advisory, not a billing system. Severity and confidence remain the enforcement contract. See the [rule catalog](docs/book/rules/index.md) for severity and fix guidance.

## Benchmark smoke tests

```bash
python3 scripts/benchmark_external_repo.py --all-vendored
python3 scripts/benchmark_external_repo.py --repo spellbook --smoke
python3 scripts/benchmark_external_repo.py --repo data-infra --smoke
python3 scripts/benchmark_external_repo.py --repo spellbook   # full gate (manual / baseline refresh)
python3 scripts/benchmark_external_repo.py --repo data-infra    # full gate (manual / baseline refresh)
cargo test -p costguard-core --test corpus
```

Layer definitions: [Benchmark tiers](docs/book/contributing/benchmark-tiers.md).

Run the PR-equivalent local gate with `./scripts/ci_local.sh`. Authoritative release qualification uses `python3 scripts/release_check.py --version <version>` and additionally requires a successful exact-SHA push CI run, a signed tag, and external evidence.

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
| `1` | Findings at or above `--fail-on` with confidence at or above `--min-confidence` when set, or estimated monthly p50 cost at or above `--fail-on-cost-delta` when set |
| `2` | Config error |
| `3` | Runtime error |

## Status

`v2.0.0` is the MVP production release. Generic SQL, Snowflake, BigQuery, and Trino are supported; Databricks, Redshift, Postgres, and DuckDB remain preview. Cost estimates are advisory, warehouse connectivity is out of scope, and manifest-backed analysis requires the caller's dbt compile step. See the [support policy](SUPPORT.md), [compatibility policy](docs/book/reference/compatibility.md), and [security policy](SECURITY.md).
