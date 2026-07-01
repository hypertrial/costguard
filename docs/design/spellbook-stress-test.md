# Dune Spellbook Stress Test Plan

**Status:** Point-in-time snapshot. **Last validated:** 2026-06-16.

Dune Spellbook (`duneanalytics/spellbook`) is the primary public-real stress test for `costguard`.

Primary repo: <https://github.com/duneanalytics/spellbook>

## Automated benchmark harness (recommended)

Use the repo benchmark script instead of ad-hoc clone commands:

```bash
python3 scripts/benchmark_external_repo.py --repo spellbook
python3 scripts/benchmark_external_repo.py --repo spellbook --smoke
python3 scripts/benchmark_external_repo.py --repo jaffle-shop
python3 scripts/benchmark_external_repo.py --all-vendored
```

The script compiles five subprojects, **merges** their manifests into root `target/manifest.json`, scans with `--warehouse trino`, and compares against baselines.

Compiled-parse gate:

```bash
cargo run -p costguard-sql --bin audit-compiled-parse --features audit-bin -- \
  ~/.cache/costguard/benchmarks/spellbook/target/manifest.json --bucket
```

Pinned commits and scan paths are defined in
[`tests/benchmarks/repos.toml`](../../tests/benchmarks/repos.toml).
Baselines live in [`tests/benchmarks/baselines/`](../../tests/benchmarks/baselines/).
See [`benchmark-calibration.md`](benchmark-calibration.md) for the triage loop and [Benchmark tiers](../book/contributing/benchmark-tiers.md) for the canonical four-layer model.

GitHub Actions:

- **Push to `main`:** Spellbook **smoke** benchmark (`tokens` + root macros) in [`ci.yml`](../../.github/workflows/ci.yml) (`spellbook-smoke` job).
- **Manual:** run the **benchmark** workflow (`workflow_dispatch`) in [`benchmark.yml`](../../.github/workflows/benchmark.yml) with target `spellbook` for the full five-subproject gate.

Why Spellbook:

- It is a real production-style dbt project, not a tutorial.
- It is public, large, active, macro-heavy, and structurally complex.
- It has `models/`, `sources/`, `dbt_macros/`, and multiple `dbt_subprojects/`.
- Its blockchain analytics domain creates realistic SQL and Jinja patterns across DEX, NFT, Solana, and token datasets.
- It should expose scanner gaps, dbt graph assumptions, noisy rules, and parser resilience issues quickly.

## Manual workflow (advanced / debugging)

For debugging outside the benchmark harness:

```bash
git clone https://github.com/duneanalytics/spellbook.git
cd spellbook

pip install dbt-trino
for sub in dex tokens solana daily_spellbook hourly_spellbook; do
  dbt deps --project-dir "dbt_subprojects/${sub}"
  dbt compile --project-dir "dbt_subprojects/${sub}" --target dev
done
```

After per-subproject compiles, merge manifests before scanning (the benchmark script does this via `merge_manifests` in `scripts/benchmark_external_repo.py`). Without a merged root manifest, `--manifest target/manifest.json` may be missing or incomplete.

Example scan after merge (or use the benchmark script):

```bash
costguard scan . --warehouse trino --manifest target/manifest.json
costguard scan dbt_subprojects --warehouse trino --manifest target/manifest.json --format json > costguard-spellbook.json
costguard scan . --warehouse trino --manifest target/manifest.json --fail-on high
```

PR check output smoke checks:

```bash
costguard pr --base origin/main --warehouse trino --manifest target/manifest.json --fail-on high --format github
costguard pr --base origin/main --warehouse trino --manifest target/manifest.json --fail-on high --format markdown
```

These commands should be run manually or in an explicit benchmark job, not in normal CI.

Later, if project-directory workflows need targeted checks:

```bash
costguard scan dbt_subprojects/dex --warehouse trino --manifest target/manifest.json
costguard scan dbt_subprojects/solana --warehouse trino --manifest target/manifest.json
costguard scan dbt_subprojects/tokens --warehouse trino --manifest target/manifest.json
```

## Metrics to capture

| Metric | Why |
| --- | --- |
| Total files scanned | scanner correctness |
| SQL/Jinja parse failure rate (model-scoped) | robustness — see [Parse metrics](../book/reference/parse-metrics.md) |
| `sql_parse_compiled_failures` | Trino compiled SQL dialect quality (Spellbook gate: 0) |
| Diagnostics per rule | noisy-rule detection |
| High-severity false positives | MVP quality |
| Runtime | Rust performance value |
| Peak memory | enterprise-scale viability |
| Suppression needs | rule ergonomics |
| Crash count | parser resilience |

## Benchmark layers

See [Benchmark tiers](../book/contributing/benchmark-tiers.md) for the canonical four-layer model. Legacy Spellbook roadmap names:

```text
tier_0_smoke:   dbt-labs/jaffle-shop        -> External layer
tier_1_real:    mattermost/mattermost-data-warehouse
tier_2_stress:  duneanalytics/spellbook     -> External layer
tier_3_breadth: selected awesome-public-dbt-projects repos
tier_4_scale:   synthetic 1k/5k/10k         -> Synthetic scale layer
```

Use Spellbook as the primary public-real stress test before expanding to the broader public dbt corpus.

## Synthetic scale harness

```bash
python3 scripts/generate_synthetic_dbt.py /tmp/costguard-synthetic-1k --models 1000
costguard scan /tmp/costguard-synthetic-1k --warehouse generic --fail-on critical

python3 scripts/generate_synthetic_dbt.py /tmp/costguard-synthetic-5k --models 5000
costguard scan /tmp/costguard-synthetic-5k --warehouse generic --fail-on critical

python3 scripts/generate_synthetic_dbt.py /tmp/costguard-synthetic-10k --models 10000
costguard scan /tmp/costguard-synthetic-10k --warehouse generic --fail-on critical
```

Use the synthetic harness to measure regex caching, project-level indexes, runtime, and peak
memory without requiring network access. Keep Spellbook as the real-world false-positive and
dbt/Jinja robustness benchmark.

## Local regression corpus

Add focused rule/regression cases under [`tests/fixtures/corpus/`](../../tests/fixtures/corpus/):

1. Create a mini dbt project directory with `models/` and optional `schema.yml`.
2. Register the case in [`tests/fixtures/corpus/manifest.toml`](../../tests/fixtures/corpus/manifest.toml) with `expect_rules` and/or `forbid_rules`.
3. Run `cargo test -p costguard-core --test corpus`.

Secondary repos integrated or planned after Spellbook:

- Mattermost data warehouse (observational): <https://github.com/mattermost/mattermost-data-warehouse/tree/master/transform/snowflake-dbt>
- NBA Monte Carlo (required, DuckDB): <https://github.com/matsonj/nba-monte-carlo> — dbt project in `transform/`, smoke scans NBA models and macros
- Tuva (required, DuckDB): <https://github.com/tuva-health/tuva> — package scan with `integration_tests` as the compile consumer
- Cal-ITP data infrastructure (observational, BigQuery): <https://github.com/cal-itp/data-infra/> — dbt project in `warehouse/`, manual scan only
- dbt Jaffle Shop smoke test: <https://github.com/dbt-labs/jaffle-shop>
- Public dbt corpus: <https://github.com/InfuseAI/awesome-public-dbt-projects>

### Cal-ITP data-infra (BigQuery external benchmark)

Use the same benchmark harness as Spellbook:

```bash
python3 scripts/benchmark_external_repo.py --repo data-infra
```

Pinned commit and scan paths are in [`tests/benchmarks/repos.toml`](../../tests/benchmarks/repos.toml). The benchmark is observational (`required = false`) and stays manual; push CI no longer runs a data-infra smoke job.
