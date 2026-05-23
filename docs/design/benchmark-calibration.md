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
python3 scripts/audit_compiled_parse_failures.py \
  ~/.cache/costguard/benchmarks/spellbook/target/manifest.json --json

# Bucket rule diagnostics for triage (requires cached Spellbook checkout + manifest)
python3 scripts/bucket_rule_diagnostics.py --repo spellbook --rule SQLCOST012
python3 scripts/bucket_rule_diagnostics.py --repo spellbook --rule SQLCOST017 --join-audit /tmp/audit.json

# Validate false-positive registry against corpus forbid_rules contracts
python3 scripts/validate_fp_registry.py

# Refresh baselines after intentional rule tuning
python3 scripts/benchmark_external_repo.py --fixture real_world/jaffle_snippets --update-baseline
python3 scripts/benchmark_external_repo.py --repo spellbook --update-baseline
```

GitHub Actions:

- **Push to `main`:** [`benchmark.yml`](../../.github/workflows/benchmark.yml) runs **Spellbook smoke** (`tokens` subproject + `dbt_macros`) only. Vendored baselines run in [`ci.yml`](../../.github/workflows/ci.yml).
- **Manual:** run the **benchmark** workflow (`workflow_dispatch`) with target `vendored`, `jaffle-shop`, `spellbook-smoke`, `spellbook`, or `all`. Full Spellbook (five subprojects) is **dispatch-only**.

Benchmark scripts build the CLI in **release** mode by default (`COSTGUARD_BUILD_PROFILE=release`). dbt manifests are cached under `{cache}/manifests/{repo}/{commit}/{packages_fp}/` and skipped on warm runs unless `--force-compile` is passed.

## Baseline files

Baselines live in [`tests/benchmarks/baselines/`](../../tests/benchmarks/baselines/). External repo pins are defined in [`tests/benchmarks/repos.toml`](../../tests/benchmarks/repos.toml).

| Target kind | Pass criteria |
| --- | --- |
| Vendored | Exact rule counts, parse failure ceiling, forbidden rules |
| External | Crash-free, model parse failures ≤ baseline + delta, optional parse failure rate cap, `max_diagnostics_by_rule` ceilings on triaged rules |

### False-positive registry

Machine-readable FP contracts live in [`tests/benchmarks/fp_registry.toml`](../../tests/benchmarks/fp_registry.toml). Each `verdict = "fp"` entry must map to a corpus case with matching `forbid_rules`. CI runs `python3 scripts/validate_fp_registry.py`.

### Cross-reference workflow

1. Run Spellbook benchmark and inspect `tests/benchmarks/reports/external__spellbook.json`.
2. Bucket noisy rules with `scripts/bucket_rule_diagnostics.py` (supports `--parse-input-filter`, `--join-audit`).
3. Audit parse failures with `audit_compiled_parse_failures.py --json` (items include `original_file_path`).
4. Extract corpus fixtures, register in `fp_registry.toml`, and refresh baselines.

### Parse metric semantics

Primary parse metrics (`sql_parse_total`, `sql_parse_failures`) count **production dbt models** only (`models/**/*.sql`). Macros, tests, and other SQL files are tracked separately as `sql_parse_other_total` / `sql_parse_other_failures`.

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

### False positive tracking template

| Rule | Repo | Verdict | Notes |
| --- | --- | --- | --- |
| SQLCOST005 | spellbook | fixed | `block_time`, `evt_block_time`, `block_date`, and related needles added |
| SQLCOST004 | spellbook | partially fixed | schema YAML, nested `dbt_project.yml`, explicit `incremental_strategy: append`, and compiled Trino parsing reduce false positives |
| SQLCOST004 | spellbook | investigate remaining | many incrementals still lack `unique_key` and explicit append strategy |
| parse metrics | spellbook | improved (tier 1) | compile + Trino + model-scoped metrics: ~67% model parse failure rate (5423/8108) |
| parse metrics | spellbook | improved (P0–P2) | five-subproject compile + Trino normalization + raw fallback: **12%** model parse failure rate (972/8108), `sql_parse_compiled_total` 8001 |
| parse metrics | spellbook | improved (compiled parse) | Trino dialect + parse-only rewrites + Generic fallback: **`sql_parse_compiled_failures` 0/8001**, headline failures **107/8108** |
| SQLCOST002 | jaffle-shop | true positive | repeated JSON extraction in staging |
| SQLCOST012 | spellbook | fixed (2026-05 pass 2) | **804 → 88** after depth-0 FROM targeting (ignore inner CTE FROM comma FPs) |
| SQLCOST005 | spellbook | fixed (2026-05 pass 2) | **247 → 1** after full-file `incremental_predicate` / config macro recognition |
| SQLCOST016 | spellbook | fixed (2026-05) | **281 → 15** after staging exempt, date_trunc whitelist, compiled AST extraction; registry + corpus `partition_date_trunc_bound` |
| SQLCOST017 | spellbook | mixed (2026-05) | symmetric lower/trim exempt + staging exempt; compiled AST increases AST-confirmed hits **819 → 1003** (more accurate, use `--min-confidence high` for PR gates) |
| SQLCOST019 | spellbook | fixed (2026-05) | **374 → 66** after whole-scope partition predicate check + CTE/JOIN ON corpus fixtures |
| parse metrics | spellbook | high severity | **2133 → 1174** after pass 2 (below ≤1800 target) |
| SQLCOST016–019 | spellbook | gated | Spellbook baseline uses `max_diagnostics_by_rule` ceilings (counts may shrink, not grow); **smoke** gate runs on `push` to `main`, full Spellbook is manual dispatch |

## PR replay testing

PR-mode behavior is covered by [`crates/costguard-core/tests/pr_replay.rs`](../../crates/costguard-core/tests/pr_replay.rs), which verifies changed-file scoping and incremental rule emission on a temp git repo.

## Related docs

- [Documentation book](../book/README.md)
- [Spellbook stress test plan](spellbook-stress-test.md)
- [PR check primary workflow](pr-check-primary-workflow.md)
