# Scripts

Helper scripts live under [`scripts/`](../../scripts/) at the repository root. Prefer `python3` when invoking them.

## `benchmark_external_repo.py`

Run vendored fixtures or clone external dbt repos at pinned commits.

```bash
# Vendored (no network)
python3 scripts/benchmark_external_repo.py --all-vendored
python3 scripts/benchmark_external_repo.py --fixture real_world/jaffle_snippets

# External (network + clone cache)
python3 scripts/benchmark_external_repo.py --repo jaffle-shop
python3 scripts/benchmark_external_repo.py --repo spellbook

# Refresh baselines after intentional rule tuning
python3 scripts/benchmark_external_repo.py --fixture real_world/jaffle_snippets --update-baseline
python3 scripts/benchmark_external_repo.py --repo spellbook --update-baseline
```

Common flags:

| Flag | Description |
| --- | --- |
| `--repo` | External repo key from `tests/benchmarks/repos.toml` |
| `--fixture` | Vendored fixture path under `tests/fixtures/` |
| `--all-vendored` | Run all vendored baselines |
| `--update-baseline` | Write report metrics to baseline JSON |
| `--warehouse` | Override scan warehouse (defaults per target) |

For Spellbook, the script compiles five subprojects and **merges** their manifests into `target/manifest.json` at the repo root before scanning.

Validate vendored baselines in Rust:

```bash
cargo test -p costguard-core --test benchmark vendored_baselines_match
```

## `audit_compiled_parse_failures.py`

Audit compiled SQL parse failures from a dbt manifest (Spellbook gate).

```bash
python3 scripts/audit_compiled_parse_failures.py path/to/manifest.json
python3 scripts/audit_compiled_parse_failures.py path/to/manifest.json --bucket
```

Builds and runs the `audit-compiled-parse` binary from `costguard-sql`.

### `audit-compiled-parse` binary

Not installed by `cargo install --path crates/costguard-cli`. Build explicitly:

```bash
cargo build -p costguard-sql --bin audit-compiled-parse
./target/debug/audit-compiled-parse [--bucket] [--model NAME] [--json] MANIFEST.json
```

| Flag | Description |
| --- | --- |
| `--bucket` | Print error signature counts |
| `--model` | Inspect a single model by name |
| `--json` | Emit JSON report |

## `generate_synthetic_dbt.py`

Generate synthetic dbt-style projects for scale testing without network access.

```bash
python3 scripts/generate_synthetic_dbt.py /tmp/costguard-synthetic-1k --models 1000
costguard scan /tmp/costguard-synthetic-1k --warehouse generic --fail-on critical

python3 scripts/generate_synthetic_dbt.py /tmp/costguard-synthetic-5k --models 5000
costguard scan /tmp/costguard-synthetic-5k --warehouse generic --fail-on critical

python3 scripts/generate_synthetic_dbt.py /tmp/costguard-synthetic-10k --models 10000
costguard scan /tmp/costguard-synthetic-10k --warehouse generic --fail-on critical
```

## `generate_rule_docs.py`

Regenerate the mdBook rule catalog from `costguard rules --format json`:

```bash
python3 scripts/generate_rule_docs.py
python3 scripts/generate_rule_docs.py --check
```

## Related

- [Benchmark tiers](../contributing/benchmark-tiers.md)
- [Benchmark calibration](../design/benchmark-calibration.md)
