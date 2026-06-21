# Benchmark tiers

Costguard validates changes through four benchmark **layers**. See the [Glossary](../glossary.md) for legacy tier name mapping.

## Layer overview

| Layer | Speed | Network | Purpose |
| --- | --- | --- | --- |
| Corpus | Fastest | No | Rule contract regression |
| Vendored | Fast | No | Production-style snippets offline |
| External | Slow | Yes | Full public repo stress tests |
| Synthetic scale | Medium | No | Runtime/memory at 1k–10k models |

## Layer 1 — Corpus

Deterministic mini dbt projects under [`tests/fixtures/corpus/`](../../../tests/fixtures/corpus/).

```bash
cargo test -p costguard-core --test corpus
python3 -m venv .venv-eval && .venv-eval/bin/pip install --require-hashes -r requirements-eval.lock
.venv-eval/bin/python scripts/eval_metrics.py --split corpus
```

Register cases in [`tests/fixtures/corpus/manifest.toml`](../../../tests/fixtures/corpus/manifest.toml). Frozen classification labels live in [`tests/benchmarks/eval_labels.toml`](../../../tests/benchmarks/eval_labels.toml); regenerate with `python3 scripts/build_eval_dataset.py --write`. See [Classification metrics](../../design/classification-metrics.md).

## Layer 2 — Vendored

Offline snippets in [`tests/fixtures/real_world/`](../../../tests/fixtures/real_world/).

```bash
python3 scripts/benchmark_external_repo.py --all-vendored
cargo test -p costguard-core --test benchmark vendored_baselines_match
```

## Layer 3 — External

Pinned clones defined in [`tests/benchmarks/repos.toml`](../../../tests/benchmarks/repos.toml). Manual TP/FP adjudication on external findings uses the [Manual rule review playbook](../../design/manual-rule-review.md); outcome scoreboard: [Rule TP coverage](../../design/rule-tp-coverage.md).

```bash
python3 scripts/benchmark_external_repo.py --repo jaffle-shop
python3 scripts/benchmark_external_repo.py --repo spellbook
python3 scripts/benchmark_external_repo.py --repo spellbook --smoke
python3 scripts/benchmark_external_repo.py --repo nba-monte-carlo
python3 scripts/benchmark_external_repo.py --repo nba-monte-carlo --smoke
python3 scripts/benchmark_external_repo.py --repo tuva
python3 scripts/benchmark_external_repo.py --repo ol-data-platform
python3 scripts/benchmark_external_repo.py --repo ol-data-platform --smoke
python3 scripts/benchmark_external_repo.py --repo data-infra  # manual observational
```

GitHub Actions workflows are optional `workflow_dispatch` mirrors. The local release gate is authoritative and runs the full pinned repositories.

### Defaults by context

| Context | warehouse | fail-on |
| --- | --- | --- |
| PR examples / MVP | `snowflake` (illustrative) | `high` |
| External: spellbook | `trino` | `critical` |
| External: jaffle-shop | `generic` | `critical` |
| External: nba-monte-carlo | `duckdb` | `critical` |
| External: tuva | `duckdb` | `critical` |
| External: ol-data-platform | `trino` | `critical` |
| External: data-infra (manual observational) | `bigquery` | `critical` |
| Vendored harness | `generic` | `critical` |
| This repo PR workflow | `generic` | `high` |

`ol-data-platform` scans with `trino` but compiles offline via DuckDB `dev` (repo-native profiles) plus a `dbt_compile_shim` injected into `trino_utils` so `dbt compile` succeeds without warehouse credentials. See `tests/benchmarks/compile-shims/ol-data-platform.sql`.

## Layer 4 — Synthetic scale

```bash
python3 scripts/generate_synthetic_dbt.py /tmp/costguard-synthetic-1k --models 1000
costguard scan /tmp/costguard-synthetic-1k --warehouse generic --fail-on critical

python3 scripts/generate_synthetic_dbt.py /tmp/costguard-synthetic-5k --models 5000
costguard scan /tmp/costguard-synthetic-5k --warehouse generic --fail-on critical

python3 scripts/generate_synthetic_dbt.py /tmp/costguard-synthetic-10k --models 10000
costguard scan /tmp/costguard-synthetic-10k --warehouse generic --fail-on critical
```

## Pass criteria summary

| Target kind | Pass criteria |
| --- | --- |
| Vendored | Exact rule counts, parse failure ceiling, forbidden rules |
| External | Crash-free, parse failures ≤ baseline + delta, optional rate cap, `sql_parse_compiled_failures` gate for Spellbook |

Details: [Benchmark calibration](../../design/benchmark-calibration.md).

## Related

- [Scripts](../reference/scripts.md)
- [Spellbook stress test](../../design/spellbook-stress-test.md)
