# dbt cost regression checks for CI

[![CI](https://img.shields.io/github/actions/workflow/status/hypertrial/costguard/ci.yml?branch=main)](https://github.com/hypertrial/costguard/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/hypertrial/costguard)](https://github.com/hypertrial/costguard/releases)
[![License: MIT](https://img.shields.io/github/license/hypertrial/costguard)](LICENSE)
[![Docs](https://img.shields.io/badge/docs-mdBook-blue)](docs/book/README.md)

Costguard reviews dbt pull requests before merge.

It scans changed models against the git base, uses optional dbt manifest and lineage context for downstream impact, and runs without warehouse credentials or live queries.

One binary and one simple CI Action. `costguard pr` is the main workflow; `costguard scan` is the local debugging path.

Measured on real dbt benchmark repos and the corpus suite:

- **97.2%** overall sampled precision
- **99.8%** high-severity sampled precision
- **44/44** behavioral rules passing TP census

Generic SQL, Snowflake, BigQuery, and Trino scanning are production-ready; Databricks, Redshift, Postgres, and DuckDB support is preview.

## Install

```bash
curl -fsSL https://raw.githubusercontent.com/hypertrial/costguard/main/scripts/install.sh | sh
```

Pin a version: `... | sh -s -- v2.5.0`. Or build from source: `cargo install --git https://github.com/hypertrial/costguard --tag v2.5.0 costguard-cli`.

See [Installation](docs/book/getting-started/installation.md) for pinned/airgapped manual install and Windows.

## Run locally

No config file or flags required. From your dbt project root:

```bash
costguard scan
```

Add `--warehouse snowflake` (or `bigquery`, `trino`, etc.) for sharper dialect-specific parsing. See [Requirements](docs/book/getting-started/requirements.md) for manifest and compile guidance.

## Add to CI

From your dbt project root:

```bash
costguard init
```

This writes `.github/workflows/costguard.yml` and a starter `costguard.toml` (best-effort warehouse detection from `dbt_project.yml` / `profiles.yml`). Commit the workflow, then open a PR.

Or add the Action manually after your existing dbt compile step:

```yaml
- uses: actions/checkout@v6
  with:
    fetch-depth: 0
- run: dbt compile --target dev
- uses: hypertrial/costguard/.github/actions/costguard@v2.5.0
  with:
    base: origin/main
    warehouse: snowflake
    fail-on: high
    min-confidence: high
    receipt-path: costguard-receipt.json
```

Pin the exact Action tag `@v2.5.0` or use the moving compatible major tag `@v2`. Release binaries are checksum-protected and include provenance attestations.

**2.1 requirements:** Use baseline v3 and policy v2 with `identity_scheme: "semantic-v1"`. Older baseline and policy schemas are rejected at scan time. See [Compatibility policy](docs/book/reference/compatibility.md).

## Requirements

Costguard reads **source files, git history, and (optionally) `target/manifest.json`**. It never connects to your warehouse and never needs credentials. The manifest is auto-detected when present; run `dbt compile` first only if you want compiled-SQL analysis on Jinja-heavy models.

Full table: [Requirements](docs/book/getting-started/requirements.md).

## Costguard vs general SQL analyzers

General SQL analyzers are broad linting tools for security, compliance, migrations, app-code SQL extraction, schema validation, autofix, and editor feedback.

Costguard is narrower by design: a dbt PR cost regression gate for changed models, downstream blast radius, severity/confidence enforcement, advisory savings, and credential-free CI.

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
| Installation | [Installation](docs/book/getting-started/installation.md) |
| Requirements | [Requirements](docs/book/getting-started/requirements.md) |
| Local scan | [Local scan and explain](docs/book/getting-started/local-scan.md) |
| Troubleshooting | [Troubleshooting](docs/book/getting-started/troubleshooting.md) |
| PR check setup | [Quick start](docs/book/getting-started/quick-start.md) |
| CLI and config | [Reference](docs/book/reference/cli.md) |
| Rule catalog | [Rules](docs/book/rules/index.md) |
| Benchmarks | [Benchmark tiers](docs/book/contributing/benchmark-tiers.md) |
| Benchmark evidence | [Measured precision/recall](docs/book/reference/benchmarks.md) |
| Terminology | [Glossary](docs/book/glossary.md) |

## GitHub Action

Use `install-mode: source` to build the checked-out Action code instead of downloading a release binary:

```yaml
- uses: hypertrial/costguard/.github/actions/costguard@main
  with:
    install-mode: source
    base: origin/main
    warehouse: snowflake
    fail-on: high
    min-confidence: high
```

The Action does not install or compile dbt. See [Requirements](docs/book/getting-started/requirements.md) for manifest and git history needs.

Only high-confidence, high-severity findings fail the PR by default. Pair `fail-on: high` with `min-confidence: high` on macro-heavy dbt repos.

See [Quick start (PR check)](docs/book/getting-started/quick-start.md) for inputs and workflow guidance.

## Example output

```text
$ costguard pr --base origin/main --warehouse snowflake --fail-on high --min-confidence high

HIGH SQLCOST006 models/marts/fct_orders.sql:42:12
  Unbounded join risk: no equality predicate on join keys
  confidence: high

HIGH SQLCOST014 models/staging/stg_events.sql:18:5
  Repeated CTE reference may multiply work
  confidence: high

2 findings (2 high, 0 medium, 0 low)
exit code: 1
```

Use `--format github` for workflow annotations. Add `--summary-file summary.md` and `--receipt-file receipt.json` to write markdown and JSON v4 from the same scan; `--compare-receipt previous.json` adds trend deltas. The Action writes its markdown step summary automatically.

## What it detects

Costguard ships **46 SQLCOST rules** for incremental safety, join risk, warehouse cost patterns, and dbt anti-patterns. Optional **[cost estimates](docs/book/reference/cost-estimates.md)** attach per-finding savings and project-level current/post-fix/potential savings for prioritization—advisory priors from local files, not a bill. Severity and confidence remain the enforcement contract. See the [rule catalog](docs/book/rules/index.md) for severity and fix guidance.

## Benchmark smoke tests

```bash
python3 scripts/benchmark_external_repo.py --all-vendored
python3 scripts/build_benchmark_evidence.py
python3 scripts/generate_precision_tiers.py
python3 scripts/benchmark_external_repo.py --repo spellbook --smoke
python3 scripts/benchmark_external_repo.py --repo nba-monte-carlo --smoke
python3 scripts/benchmark_external_repo.py --repo spellbook   # full gate (manual / baseline refresh)
python3 scripts/benchmark_external_repo.py --repo nba-monte-carlo
python3 scripts/benchmark_external_repo.py --repo tuva
python3 scripts/benchmark_external_repo.py --repo ol-data-platform
python3 scripts/benchmark_external_repo.py --repo data-infra    # manual observational
cargo test -p costguard-core --test corpus
```

Layer definitions: [Benchmark tiers](docs/book/contributing/benchmark-tiers.md).

Run the PR-equivalent local gate with `./scripts/ci_local.sh`. Authoritative release qualification uses `python3 scripts/release_check.py --version <version>` and additionally requires a successful exact-SHA push CI run, a signed tag, and external evidence.

## Configuration sketch

`costguard.toml` is optional. Costguard runs with sensible defaults when the file is absent; the example below shows tunable knobs only.

```toml
warehouse = "snowflake"

[scan]
paths = ["models"]
ignore = ["target", "dbt_packages"]

[output]
fail_on = "high"

[dbt]
manifest_path = "target/manifest.json"

[owners]
codeowners = true
default = "@data-platform"

[gate]
fail_on = "high"
min_confidence = "high"
require_owner = true
```

Full schema: [Configuration](docs/book/reference/configuration.md).

## Exit codes

| Code | Meaning |
| --- | --- |
| `0` | Pass |
| `1` | Analysis incomplete, a blocking PR gate failed, findings met `--fail-on`/`--min-confidence`, or an enabled cost threshold was exceeded |
| `2` | Config error |
| `3` | Runtime error |

## Status

`v2.5.0` adds owner-aware routing, scoped PR gates, expiring local waivers controlled by signed policy, one-scan markdown/JSON receipts with trend comparison, and explicit warehouse-prior provenance. JSON schema v4, baseline v3, policy v2, rule IDs, and default exit behavior remain compatible. `v2.4.0` added lineage-aware downstream cost propagation, warehouse cost priors, committed [benchmark evidence](docs/book/reference/benchmarks.md), and measured precision tiers. Generic SQL, Snowflake, BigQuery, and Trino are supported; Databricks, Redshift, Postgres, and DuckDB remain preview. Cost estimates are advisory, warehouse connectivity is out of scope, and manifest-backed analysis requires the caller's dbt compile step. See the [support policy](SUPPORT.md), [compatibility policy](docs/book/reference/compatibility.md), and [security policy](SECURITY.md).

Licensed under [MIT](LICENSE). Contributions welcome — see [CONTRIBUTING.md](CONTRIBUTING.md).
