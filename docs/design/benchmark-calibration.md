# Benchmark calibration workflow

Costguard uses a **four-layer** benchmark model for realistic dbt project analysis. See the [Glossary — Benchmark layers](../book/glossary.md#benchmark-layers-canonical) for canonical definitions and legacy tier mapping.

1. **Corpus regression** (`tests/fixtures/corpus/`) — fast, deterministic rule contracts in normal CI.
2. **Vendored real-world snippets** (`tests/fixtures/real_world/`) — offline production-style SQL/Jinja/manifest patterns.
3. **External repo benchmarks** — opt-in clones of public dbt projects at pinned commits.
4. **Synthetic scale** — generated 1k/5k/10k model repos for runtime and memory testing.

## Quick commands

```bash
# Vendored fixtures (no network)
python3 scripts/benchmark_external_repo.py --all-vendored
cargo test -p costguard-core --test benchmark vendored_baselines_match

# External repos (network + clone cache)
python3 scripts/benchmark_external_repo.py --repo jaffle-shop
python3 scripts/benchmark_external_repo.py --repo spellbook
python3 scripts/benchmark_external_repo.py --repo spellbook --smoke

# Force recompile (bypass manifest cache)
python3 scripts/benchmark_external_repo.py --repo spellbook --force-compile

# Audit compiled parse failures (Spellbook manifest gate)
cargo run -p costguard-sql --bin audit-compiled-parse --features audit-bin -- \
  ~/.cache/costguard/benchmarks/spellbook/target/manifest.json --json

# Bucket rule diagnostics for triage (requires cached Spellbook checkout + manifest)
python3 scripts/bucket_rule_diagnostics.py --repo spellbook --rule SQLCOST012
python3 scripts/bucket_rule_diagnostics.py --repo spellbook --rule SQLCOST017 --join-audit /tmp/audit.json

# Validate false-positive registry against corpus forbid_rules contracts
python3 scripts/validate_fp_registry.py

# Recall coverage gate (>=2 expect_rules and >=1 forbid_rules per behavioral rule)
python3 scripts/recall_report.py

# Corpus binary-classification metrics (precision/recall/F1/MCC/PR-AUC)
python3 -m venv .venv-eval && .venv-eval/bin/pip install -r requirements-eval.txt
.venv-eval/bin/python scripts/eval_metrics.py --split corpus
.venv-eval/bin/python scripts/eval_irr.py

# Sampled precision report (requires cached Spellbook checkout + manifest)
python3 scripts/precision_triage.py --repo spellbook --sample-size 200

# Per-rule TP census across all benchmark repos (44-rule pass bar)
python3 scripts/rule_tp_census.py --emit-evidence

# Cost-ranked finding review packets (Spellbook or other repo)
python3 scripts/top_findings_review.py --repo spellbook --top 50

# Refresh baselines after intentional rule tuning
python3 scripts/benchmark_external_repo.py --fixture real_world/jaffle_snippets --update-baseline
python3 scripts/benchmark_external_repo.py --repo spellbook --update-baseline
```

GitHub Actions:

- **Push to `main`:** [`benchmark.yml`](../../.github/workflows/benchmark.yml) runs **Spellbook smoke** (`tokens` subproject + `dbt_macros`) only. Vendored baselines run in [`ci.yml`](../../.github/workflows/ci.yml).
- **Manual:** run the **benchmark** workflow (`workflow_dispatch`) with target `vendored`, `jaffle-shop`, `spellbook-smoke`, `spellbook`, `precision`, or `all`. Full Spellbook (five subprojects) is **dispatch-only**. The `precision` target runs full Spellbook plus `precision_triage.py` gates.

Benchmark scripts build the CLI in **release** mode by default (`COSTGUARD_BUILD_PROFILE=release`). dbt manifests are cached under `{cache}/manifests/{repo}/{commit}/{packages_fp}/` and skipped on warm runs unless `--force-compile` is passed.

## Baseline files

Baselines live in [`tests/benchmarks/baselines/`](../../tests/benchmarks/baselines/). External repo pins are defined in [`tests/benchmarks/repos.toml`](../../tests/benchmarks/repos.toml).

| Target kind | Pass criteria |
| --- | --- |
| Vendored | Exact rule counts, parse failure ceiling, forbidden rules |
| External | Crash-free, model parse failures ≤ baseline + delta, optional parse failure rate cap, `max_diagnostics_by_rule` ceilings on triaged rules, median/maximum runtime, and peak RSS |

### Enterprise readiness gates (Spellbook)

| Gate | Threshold |
| --- | --- |
| Model parse failures | 0% headline rate; `sql_parse_compiled_failures = 0` |
| High-severity sampled precision | ≥ 90% (`scripts/precision_triage.py`) |
| Overall sampled precision | ≥ 80% |
| Per-rule sampled precision | ≥ 70% for each classified rule |
| Full scan runtime | One warmup plus three measured runs; median ≤15 s, maximum ≤20 s, and peak RSS ≤1 GiB |
| Baseline workflow | Rescan with `--baseline` reports 0 new findings on unchanged tree |

### False-positive registry

Machine-readable FP contracts live in [`tests/benchmarks/fp_registry.toml`](../../tests/benchmarks/fp_registry.toml). Each `verdict = "fp"` entry must map to a corpus case with matching `forbid_rules`; each `verdict = "tp"` entry must map to a corpus case with matching `expect_rules`. CI runs `python3 scripts/validate_fp_registry.py` and `python3 scripts/recall_report.py` (minimum positive/negative corpus coverage per behavioral rule).

### Cross-reference workflow

Canonical step-by-step adjudication: [Manual rule review playbook](manual-rule-review.md).

1. Run Spellbook benchmark and inspect `tests/benchmarks/reports/external__spellbook.json`.
2. Bucket noisy rules with `scripts/bucket_rule_diagnostics.py` (supports `--parse-input-filter`, `--join-audit`).
3. Audit parse failures with `cargo run -p costguard-sql --bin audit-compiled-parse --features audit-bin -- MANIFEST.json --json` (items include `original_file_path`).
4. Adjudicate findings (TP/FP), then extract corpus fixtures, register in `fp_registry.toml`, and refresh baselines.
5. Confirm 44/44 census PASS: `python3 scripts/rule_tp_census.py --emit-evidence` — see [Rule TP coverage](rule-tp-coverage.md).

### Parse metric semantics

Primary parse metrics (`sql_parse_total`, `sql_parse_failures`) count **production dbt models** only (`models/**/*.sql`, excluding `macros/models/**` macro templates). Macros, tests, and other SQL files are tracked separately as `sql_parse_other_total` / `sql_parse_other_failures`.

When a manifest with `compiled_code` is loaded:

- Costguard normalizes compiled SQL (comment stripping, Trino rewrites, GenericDialect fallback) before parse attempts.
- Headline `sql_parse_failures` uses compiled parse when available, with **stripped-raw fallback** when compiled parse fails (`parsed_compiled || parsed_raw`).
- `sql_parse_compiled_total` counts models with a compiled attempt; `sql_parse_compiled_failures` counts models where **compiled parse failed** (dialect quality signal, independent of raw fallback). Spellbook baseline requires **`sql_parse_compiled_failures = 0`**.

External Spellbook benchmarks compile all five subprojects (`dex`, `tokens`, `solana`, `daily_spellbook`, `hourly_spellbook`) — see `dbt_compile_dirs` in [`repos.toml`](../../tests/benchmarks/repos.toml) — merge manifests into root `target/manifest.json`, and scan with `--warehouse trino`.

Reports are written to `tests/benchmarks/reports/` (gitignored).

## Calibration loop

When an external benchmark surfaces a finding worth keeping:

1. **Triage** the diagnostic — true positive, false positive, or parser limitation.
2. **Extract** a minimal SQL/YAML snippet into `tests/fixtures/corpus/` or `tests/fixtures/real_world/`.
3. **Register** corpus cases in [`tests/fixtures/corpus/manifest.toml`](../../tests/fixtures/corpus/manifest.toml).
4. **Update** the relevant baseline with `--update-baseline`.
5. **Record** the verdict in the table below.

### Latest review pass (2026-06)

| Milestone | Outcome | Doc |
| --- | --- | --- |
| Spellbook top-250 cost triage | 250/250 registry TP on cost-ranked queue | [Spellbook top-10 cost review](spellbook-top10-cost-review.md) |
| 44-rule census (4 repos) | 44/44 PASS | [Rule TP coverage](rule-tp-coverage.md) |
| Manual review playbook | Canonical workflow | [Manual rule review](manual-rule-review.md) |

Historical parse-metric and per-rule FP counts from 2025–2026 passes are archived in git history for `benchmark-calibration.md`; use the playbook for current triage procedure.

## PR replay testing

PR-mode behavior is covered by [`crates/costguard-core/tests/pr_replay.rs`](../../crates/costguard-core/tests/pr_replay.rs), which verifies changed-file scoping and incremental rule emission on a temp git repo.

## Related docs

- [Manual rule review playbook](manual-rule-review.md)
- [Rule TP coverage](rule-tp-coverage.md)
- [Documentation book](../book/README.md)
- [Spellbook stress test plan](spellbook-stress-test.md)
- [PR check primary workflow](pr-check-primary-workflow.md)
