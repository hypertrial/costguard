# Scripts

Helper scripts live under [`scripts/`](../../scripts/) at the repository root. Prefer `python3` when invoking them.

For Spellbook, the script compiles five subprojects and **merges** their manifests into `target/manifest.json` at the repo root before scanning. The GitHub Action uses the same merge logic via [`dbt_compile_for_costguard.py`](../../scripts/dbt_compile_for_costguard.py).

## `dbt_compile_for_costguard.py`

Shared dbt compile and manifest merge helper used by the GitHub Action and `benchmark_external_repo.py`. Subproject compiles run in parallel when multiple `--compile-dirs` are provided (`COSTGUARD_DBT_COMPILE_JOBS=1` forces serial). Manifest outputs are cached per repo commit and packages fingerprint when `--cache-dir` is set from the benchmark script.

```bash
python3 scripts/dbt_compile_for_costguard.py \
  --checkout . \
  --project-dir dbt_subprojects/dex \
  --adapter-package dbt-trino \
  --profile-type trino \
  --requirements-file requirements.txt \
  --constraints-file constraints.txt \
  --vars '{days: 7}' \
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
| `--cache-dir` | Benchmark cache root (manifest fingerprint cache when used from benchmark script) |
| `--requirements-file` | Optional pip requirements file for dbt dependencies |
| `--constraints-file` | Optional pip constraints file for reproducible dbt installs |
| `--vars` | Optional YAML string passed to `dbt compile --vars` |
| `--fail-on-deps-failure` | Exit when `dbt deps` fails instead of warning and continuing |
| `--use-existing-manifest` | Skip compile and require `--manifest-out` to already exist |

## `costguard_tooling.py`

Shared helper for locating/building the CLI. Benchmark and doc scripts default to **release** builds (`COSTGUARD_BUILD_PROFILE=release`; set `debug` for local debugging). Skips rebuild when the binary is newer than Rust sources under `crates/`.

## `verify_release_assets.py`

Builds a host-platform release tarball using the same layout as [`.github/workflows/release.yml`](../../.github/workflows/release.yml), verifies the `.sha256` checksum, and smoke-tests the extracted `costguard rules --format json` binary. CI runs this after every release build so packaging stays aligned with the GitHub Action `install-mode: release` downloader.

```bash
python3 scripts/verify_release_assets.py
```

### Cutting `v0.1.0` (operator checklist)

1. Merge the release-readiness PR to `main`.
2. Ensure GitHub Actions billing allows workflow runs.
3. Tag and push: `git tag v0.1.0 && git push origin v0.1.0`.
4. Confirm the **release** workflow uploads four `costguard-*.tar.gz` assets and matching `.sha256` files.
5. Smoke-test consumer install with `uses: hypertrial/costguard/.github/actions/costguard@v0.1.0` and default `install-mode: release` on `ubuntu-latest`.
6. Rollback: delete the GitHub release and tag if assets are bad; docs stay pinned until you re-tag.

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
python3 scripts/benchmark_external_repo.py --repo spellbook --smoke

# Refresh baselines after intentional rule tuning
python3 scripts/benchmark_external_repo.py --fixture real_world/jaffle_snippets --update-baseline
python3 scripts/benchmark_external_repo.py --repo spellbook --update-baseline
python3 scripts/benchmark_external_repo.py --repo spellbook --smoke --update-baseline
```

Common flags:

| Flag | Description |
| --- | --- |
| `--repo` | External repo key from `tests/benchmarks/repos.toml` |
| `--fixture` | Vendored fixture path under `tests/fixtures/` |
| `--all-vendored` | Run all vendored baselines |
| `--update-baseline` | Write report metrics to baseline JSON |
| `--smoke` | Run repo smoke profile (`smoke_*` keys in `repos.toml`; Spellbook: `tokens` + `dbt_macros`) |
| `--force-compile` | Bypass cached dbt manifest and recompile |
| `--warehouse` | Override scan warehouse (defaults per target) |

Reports include `compile_cache: hit|miss|skip` when dbt compile is enabled. Benchmark scripts use release CLI builds via `costguard_tooling.py`.

Validate vendored baselines in Rust:

```bash
cargo test -p costguard-core --test benchmark vendored_baselines_match
```

## `audit_compiled_parse_failures.py`

Audit compiled SQL parse failures from a dbt manifest (Spellbook gate).

```bash
python3 scripts/audit_compiled_parse_failures.py path/to/manifest.json
python3 scripts/audit_compiled_parse_failures.py path/to/manifest.json --bucket
python3 scripts/audit_compiled_parse_failures.py path/to/manifest.json --json
```

Builds and runs the `audit-compiled-parse` binary from `costguard-sql`.

### `audit-compiled-parse` binary

Not installed by `cargo install --path crates/costguard-cli`. Build explicitly:

```bash
cargo build -p costguard-sql --bin audit-compiled-parse --features audit-bin
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

Bucket per-file diagnostics for external-repo triage after running the Spellbook benchmark. Classifiers are registered for `SQLCOST012`, `SQLCOST016`, `SQLCOST017`, `SQLCOST019`, and `SQLCOST005`.

```bash
python3 scripts/bucket_rule_diagnostics.py --repo spellbook --rule SQLCOST012
python3 scripts/bucket_rule_diagnostics.py --repo spellbook --rule SQLCOST017 \
  --join-audit /tmp/audit.json --parse-input-filter compiled_with_raw_fallback \
  --json-out triage/sqlcost017.json
```

| Flag | Description |
| --- | --- |
| `--repo` | External repo key (default `spellbook`) |
| `--rule` | Rule id to bucket |
| `--limit` | Max diagnostics to classify |
| `--cache` | Benchmark cache root |
| `--join-audit` | Attach audit JSON error signatures by file path |
| `--parse-input-filter` | Filter to files with a given `parse_input` from scan JSON |
| `--json-out` | Write bucket report JSON |

Requires a cached checkout with `target/manifest.json` from `benchmark_external_repo.py --repo spellbook`.

## `validate_fp_registry.py`

Validate [`tests/benchmarks/fp_registry.toml`](../../tests/benchmarks/fp_registry.toml) against corpus `forbid_rules` contracts:

```bash
python3 scripts/validate_fp_registry.py
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
