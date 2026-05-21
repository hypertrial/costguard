# Benchmark calibration workflow

Costguard uses a three-tier testing model for realistic dbt project analysis:

1. **Corpus regression** (`tests/fixtures/corpus/`) — fast, deterministic rule contracts in normal CI.
2. **Vendored real-world snippets** (`tests/fixtures/real_world/`) — offline production-style SQL/Jinja/manifest patterns.
3. **External repo benchmarks** — opt-in clones of public dbt projects at pinned commits.

## Quick commands

```bash
# Vendored fixtures (no network)
python3 scripts/benchmark_external_repo.py --all-vendored
cargo test -p costguard-core --test benchmark vendored_baselines_match

# External repos (network + clone cache)
python3 scripts/benchmark_external_repo.py --repo jaffle-shop
python3 scripts/benchmark_external_repo.py --repo spellbook

# Refresh baselines after intentional rule tuning
python3 scripts/benchmark_external_repo.py --fixture real_world/jaffle_snippets --update-baseline
python3 scripts/benchmark_external_repo.py --repo spellbook --update-baseline
```

GitHub Actions: run the **benchmark** workflow manually (`workflow_dispatch`) with target `vendored`, `jaffle-shop`, `spellbook`, or `all`.

## Baseline files

Baselines live in [`tests/benchmarks/baselines/`](../tests/benchmarks/baselines/). External repo pins are defined in [`tests/benchmarks/repos.toml`](../tests/benchmarks/repos.toml).

| Target kind | Pass criteria |
| --- | --- |
| Vendored | Exact rule counts, parse failure ceiling, forbidden rules |
| External | Crash-free, parse failures ≤ baseline + delta |

Reports are written to `tests/benchmarks/reports/` (gitignored).

## Calibration loop

When an external benchmark surfaces a finding worth keeping:

1. **Triage** the diagnostic — true positive, false positive, or parser limitation.
2. **Extract** a minimal SQL/YAML snippet into `tests/fixtures/corpus/` or `tests/fixtures/real_world/`.
3. **Register** corpus cases in [`tests/fixtures/corpus/manifest.toml`](../tests/fixtures/corpus/manifest.toml).
4. **Update** the relevant baseline with `--update-baseline`.
5. **Record** the verdict in the table below.

### False positive tracking template

| Rule | Repo | Verdict | Notes |
| --- | --- | --- | --- |
| SQLCOST005 | spellbook | investigate | `block_time` predicates may need needle tuning |
| SQLCOST002 | jaffle-shop | true positive | repeated JSON extraction in staging |
| | | | |

## PR replay testing

PR-mode behavior is covered by [`crates/costguard-core/tests/pr_replay.rs`](../../crates/costguard-core/tests/pr_replay.rs), which verifies changed-file scoping and incremental rule emission on a temp git repo.

## Related docs

- [Spellbook stress test plan](spellbook-stress-test.md)
- [PR check primary workflow](pr-check-primary-workflow.md)
