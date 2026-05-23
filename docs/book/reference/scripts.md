# Scripts

Helper scripts live under [`scripts/`](../../scripts/) at the repository root. Prefer `python3` when invoking them.

For Spellbook, the script compiles five subprojects and **merges** their manifests into `target/manifest.json` at the repo root before scanning. The GitHub Action uses the same merge logic via [`dbt_compile_for_costguard.py`](../../scripts/dbt_compile_for_costguard.py).

## `dbt_compile_for_costguard.py`

Shared dbt compile and manifest merge helper used by the GitHub Action and `benchmark_external_repo.py`.

```bash
python3 scripts/dbt_compile_for_costguard.py \
  --checkout . \
  --project-dir dbt_subprojects/dex \
  --adapter-package dbt-trino \
  --profile-type trino \
  --manifest-out target/manifest.json

python3 scripts/dbt_compile_for_costguard.py \
  --checkout . \
  --compile-dirs "dbt_subprojects/dex,dbt_subprojects/tokens" \
  --adapter-package dbt-trino \
  --manifest-out target/manifest.json
```

| Flag | Description |
| --- | --- |
| `--checkout` | Repository root |
| `--project-dir` | Single dbt project directory (relative to checkout) |
| `--compile-dirs` | Comma/newline separated subproject paths to compile and merge |
| `--adapter-package` | pip package (for example `dbt-trino`) |
| `--profile-type` | Dummy profile adapter type (defaults from adapter package) |
| `--manifest-out` | Output path for merged or single manifest |
| `--use-system-dbt` | Use `dbt` from PATH instead of cached venv |

Unit tests:

```bash
python3 -m unittest discover -s scripts/tests -p 'test_*.py'
```

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

## `bucket_rule_diagnostics.py`

Bucket per-file diagnostics for external-repo triage after running the Spellbook benchmark:

```bash
python3 scripts/bucket_rule_diagnostics.py --repo spellbook --rule SQLCOST012
python3 scripts/bucket_rule_diagnostics.py --repo spellbook --rule SQLCOST012 \
  --json-out triage/sqlcost012.json
```

Requires a cached checkout with `target/manifest.json` from `benchmark_external_repo.py --repo spellbook`.

## `generate_rule_docs.py`

Regenerate the mdBook rule catalog from `costguard rules --format json`:

```bash
python3 scripts/generate_rule_docs.py
python3 scripts/generate_rule_docs.py --check
```

## Related

- [Benchmark tiers](../contributing/benchmark-tiers.md)
- [Benchmark calibration](../design/benchmark-calibration.md)
