# Stop expensive data-model regressions before merge.

[![CI](https://img.shields.io/github/actions/workflow/status/hypertrial/costguard/ci.yml?branch=main)](https://github.com/hypertrial/costguard/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/hypertrial/costguard)](https://github.com/hypertrial/costguard/releases)
[![License: MIT](https://img.shields.io/github/license/hypertrial/costguard)](LICENSE)
[![Docs](https://img.shields.io/badge/docs-mdBook-blue)](docs/book/README.md)

SlowQL finds SQL problems. Costguard governs dbt and Rocky cost changes.

Costguard reviews analytics pull requests before merge and gates only cost findings introduced or regressed by the change.

It scans changed models against the git base, uses optional dbt manifests and sealed Rocky compile artifacts for framework-qualified lineage, and runs without warehouse credentials or live queries.

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

Pin a version: `... | sh -s -- v2.6.0`. Or build from source: `cargo install --git https://github.com/hypertrial/costguard --tag v2.6.0 costguard-cli`.

See [Installation](docs/book/getting-started/installation.md) for pinned/airgapped manual install and Windows.

## Run locally

No config file or flags required. From your project root:

```bash
costguard scan
```

Add `--warehouse snowflake` (or `bigquery`, `trino`, etc.) for sharper dialect-specific parsing. Rocky projects compile and seal expanded SQL before scanning; see [Rocky integration](docs/book/getting-started/rocky.md).

## Add to CI

From your analytics project root:

```bash
costguard init
```

This writes `.github/workflows/costguard.yml` and a starter `costguard.toml` (best-effort warehouse detection from `dbt_project.yml` / `profiles.yml`). Commit the workflow, then open a PR.

For local dbt DuckDB projects, including repos where dbt lives under `dbt/`, use `costguard init --profile local-duckdb --dbt-dir dbt`.

Or add the Action manually after your existing dbt compile step:

```yaml
- uses: actions/checkout@v6
  with:
    fetch-depth: 0
- run: dbt compile --target dev
- uses: hypertrial/costguard/.github/actions/costguard@v2.6.0
  with:
    base: origin/main
    warehouse: snowflake
    fail-on: high
    min-confidence: high
    block-only-new: true
    receipt-path: costguard-receipt.json
```

Pin the exact Action tag `@v2.6.0` or use the moving compatible major tag `@v2`. Release binaries are checksum-protected and include provenance attestations.

**2.1 requirements:** Use baseline v3 and policy v2 with `identity_scheme: "semantic-v1"`. Older baseline and policy schemas are rejected at scan time. See [Compatibility policy](docs/book/reference/compatibility.md).

## Requirements

Costguard reads **source files, git history, and optional dbt/Rocky compile metadata**. It never connects to your warehouse and never needs credentials. dbt manifests and sealed Rocky head artifacts are auto-detected when present; Costguard invokes neither compiler.

Full table: [Requirements](docs/book/getting-started/requirements.md).

## Costguard vs SlowQL

SlowQL is a broad SQL analyzer for security, compliance, reliability, quality, performance, and cost findings. It also provides schema-aware checks, safe autofix, custom rules, and editor integration.

Costguard is narrower by design: it compares the dbt base and head, measures introduced and avoided cost, applies owner and lineage-aware change controls, and preserves a versioned evidence receipt. See the dated [Costguard vs SlowQL comparison](docs/book/reference/costguard-vs-slowql.md).

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
| Rocky | [Rocky integration](docs/book/getting-started/rocky.md) |
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

The Action does not install or compile dbt or Rocky. See [Requirements](docs/book/getting-started/requirements.md) for metadata and git history needs.

The Action defaults to regression-only enforcement: unchanged findings remain visible as notices but do not fail the PR. Pair `fail-on: high` with `min-confidence: high` on macro-heavy dbt repos.

Set `pr-comment: true` with `github-token: ${{ github.token }}` and `pull-requests: write` permission to maintain one sticky PR summary. `costguard init` enables this for newly generated workflows; existing Action consumers remain opted out.

See [Quick start (PR check)](docs/book/getting-started/quick-start.md) for inputs and workflow guidance.

## Example output

```text
# Costguard failed this PR

PR Cost Impact
- Net: +$1,240/mo
- Introduced: +$1,860/mo
- Avoided: -$620/mo
- Coverage: 84% mapped spend

Finding delta: 1 introduced, 1 regressed, 2 resolved, 7 unchanged
Changed model: model.analytics.fct_orders (owner: @finance-data)
Lineage impact: 14 downstream models, 2 exposures
Gate: default — fail; net PR increase meets $1,000/mo threshold
Receipt: JSON schema v4, receipt version 2
```

Use `--format github` for workflow annotations. Add `--summary-file summary.md` and `--receipt-file receipt.json` to write markdown and JSON v4 from the same scan; `--compare-receipt previous.json` adds trend deltas. The Action writes its markdown step summary automatically.

## What it detects

Costguard ships **47 SQLCOST rules** for incremental safety, join risk, warehouse cost patterns, framework configuration, and metadata integrity. Optional **[cost estimates](docs/book/reference/cost-estimates.md)** attach per-finding savings and project-level current/post-fix/potential savings for prioritization—advisory priors from local files, not a bill. Severity and confidence remain the enforcement contract. See the [rule catalog](docs/book/rules/index.md) for severity and fix guidance.

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

Run the full local qualification gate with `./scripts/ci_local.sh`; GitHub uses its faster `--fast` subset plus the scale gate for per-change feedback. Authoritative release qualification uses `python3 scripts/release_check.py --version <version>` and additionally requires a successful exact-SHA push CI run, a signed tag, and independently dispatched full benchmark evidence.

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
max_manifest_bytes = 536870912

[owners]
codeowners = true
default = "@data-platform"

[gate]
fail_on = "high"
min_confidence = "high"
require_owner = true
block_only_new = true
# fail_on_pr_cost_increase = 1000  # requires priced [cost]
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

`v2.6.0` adds local pipeline observation normalization, a `local-duckdb` init profile, regression-only PR gates, one-scan markdown/JSON receipts with trend comparison, and signed-policy/owner routing improvements. JSON schema v4, baseline v3, policy v2, rule IDs, and default exit behavior remain compatible. `v2.4.0` added lineage-aware downstream cost propagation, warehouse cost priors, committed [benchmark evidence](docs/book/reference/benchmarks.md), and measured precision tiers. Generic SQL, Snowflake, BigQuery, and Trino are supported; Databricks, Redshift, Postgres, and DuckDB remain preview. Cost estimates are advisory, warehouse connectivity is out of scope, and manifest-backed analysis requires the caller's dbt compile step. See the [support policy](SUPPORT.md), [compatibility policy](docs/book/reference/compatibility.md), and [security policy](SECURITY.md).

Licensed under [MIT](LICENSE). Contributions welcome — see [CONTRIBUTING.md](CONTRIBUTING.md).
