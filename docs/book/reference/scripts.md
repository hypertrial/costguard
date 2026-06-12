# Scripts

Helper scripts live under [`scripts/`](../../../scripts/) at the repository root. Prefer `python3` when invoking them.

For Spellbook, the script compiles five subprojects and **merges** their manifests into `target/manifest.json` at the repo root before scanning. The GitHub Action uses the same merge logic via [`dbt_compile_for_costguard.py`](../../../scripts/dbt_compile_for_costguard.py).

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

## `release_check.py`

Authoritative pre-release qualification gate. It requires the verified signed version tag at `HEAD`, validates the requested workspace version, runs local CI and consumer Action tests, executes pinned external benchmarks, enforces the 10,000-model performance budget, checks external documentation links, and writes `dist/release/release-check.json` bound to the commit.

```bash
python3 scripts/release_check.py --version 1.0.0
```

`--development`, `--skip-external`, and `--skip-external-links` are development aids. Development mode does not write a release qualification receipt. Strict qualification also requires `mdbook` and `cargo-deny` so documentation and dependency policy checks cannot be silently skipped.

## `verify_release_assets.py`

Builds a host-platform release tarball using the same layout as [`.github/workflows/release.yml`](../../../.github/workflows/release.yml), verifies its checksum, and smoke-tests the extracted binary. The local release gate runs this before publication.

```bash
python3 scripts/verify_release_assets.py
```

## `publish_release_local.py`

Strict local release publisher for when GitHub Actions credits are unavailable. It requires a clean checkout at a verified signed annotated tag, builds all four release targets, creates deterministic archives, validates native smoke receipts, signs provenance, and creates a new immutable GitHub release.

```bash
./scripts/publish_release_local.sh --package-only --version 1.0.0
./scripts/publish_release_local.sh --publish --version 1.0.0 \
  --receipt /path/to/smoke-x86_64-pc-windows-msvc.json
```

| Flag | Description |
| --- | --- |
| `--package-only` | Build deterministic assets and available local smoke receipts for inspection and Windows transfer |
| `--publish` | Upload assets with `gh` after all four targets pass |
| `--version` | Required version; must equal the workspace version and signed tag |
| `--workdir` | Output directory (default `dist/release`) |
| `--notes-file` | Optional release notes for `gh release create` |
| `--receipt` | Native smoke receipt; supply the Windows receipt when publishing elsewhere |
| `--qualification-receipt` | Qualification evidence (default `WORKDIR/release-check.json`) |

### Cross-compile toolchain matrix (strict all-target builds)

| Target | Typical build host | Setup |
| --- | --- | --- |
| `aarch64-apple-darwin` | Apple Silicon Mac | Native |
| `x86_64-apple-darwin` | macOS | `rustup target add x86_64-apple-darwin` |
| `x86_64-unknown-linux-gnu` | macOS/Linux | `rustup target add x86_64-unknown-linux-gnu`, install [Zig](https://ziglang.org/download/), and `cargo install cargo-zigbuild` |
| `x86_64-pc-windows-msvc` | macOS/Linux | `rustup target add x86_64-pc-windows-msvc`, `cargo install cargo-xwin`, and `cargo xwin cache xwin` |

## `smoke_release_asset.py`

Runs `--version` and `rules --format json` from an extracted native release binary and writes a receipt bound to the archive SHA-256. Windows publication requires a receipt produced on Windows.

```bash
python3 scripts/smoke_release_asset.py \
  --asset costguard-x86_64-pc-windows-msvc.tar.gz \
  --target x86_64-pc-windows-msvc \
  --version 1.0.0 \
  --receipt smoke-x86_64-pc-windows-msvc.json
```

### No Actions credits (operator checklist)

1. Install the cross toolchains from the matrix above.
2. Create the signed exact tag and qualify locally: `python3 scripts/release_check.py --version 1.0.0`.
3. Package with `--package-only`, inspect `SHA256SUMS`, and send the Windows archive for native smoke testing.
4. Follow the [release checklist](../contributing/releasing.md), including the returned Windows receipt.
5. Publish with `--publish`; it validates qualification, creates signed provenance, and rejects existing releases.
6. Smoke-test `uses: hypertrial/costguard/.github/actions/costguard@v1.0.0`, then move `v1`.

Hosted workflows are manual mirrors only and never publish a release.

## `ci_local.sh`

Authoritative local gate mirrored by [`.github/workflows/ci.yml`](../../../.github/workflows/ci.yml):

```bash
./scripts/ci_local.sh
./scripts/ci_local.sh --spellbook-smoke
```

Unit tests:

```bash
python3 -m unittest discover -s scripts/tests -p 'test_*.py'
```

## `check_docs.py`

Validates repository-local Markdown links during every local CI run. Release qualification adds retrying external URL checks with `--external`.

## `scale_check.py`

Generates and scans 10,000 clean models in release mode. The default budget is 10 seconds and 1 GiB maximum RSS with zero parse failures and zero diagnostics.

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

## `precision_triage.py`

Sample external-repo findings and compute precision against [`fp_registry.toml`](../../../tests/benchmarks/fp_registry.toml) bucket verdicts. Used for Spellbook enterprise readiness gates (≥90% high, ≥80% overall).

```bash
python3 scripts/precision_triage.py --repo spellbook --sample-size 200
python3 scripts/precision_triage.py --scan-json report.json --json-out triage/precision.json
```

| Flag | Description |
| --- | --- |
| `--repo` | External repo key (default `spellbook`) |
| `--scan-json` | Optional precomputed Costguard JSON (otherwise runs scan) |
| `--checkout` | Repo checkout path (default benchmark cache) |
| `--sample-size` | Stratified sample size (default `200`) |
| `--seed` | RNG seed for reproducible sampling |
| `--json-out` | Write precision report JSON |

Exit code `1` when precision gates fail.

## `recall_report.py`

Validate corpus recall coverage for behavioral rules (SQLCOST001–022 and SQLCOST028–035): at least two `expect_rules` cases and one `forbid_rules` case per rule in [`tests/fixtures/corpus/manifest.toml`](../../../tests/fixtures/corpus/manifest.toml).

```bash
python3 scripts/recall_report.py
python3 scripts/recall_report.py --rules SQLCOST030 SQLCOST031
```

Exit code `1` when any checked rule falls below the minimum case counts.

## `calibrate_cost_model.py`

Calibrate compute conversion factors and validate 80% interval coverage from an offline query-history CSV export:

```bash
python3 scripts/calibrate_cost_model.py exports/jobs_30d.csv
python3 scripts/calibrate_cost_model.py exports/jobs_30d.csv --json
```

Exit code `1` when coverage falls outside the 60–95% target band.

## `validate_fp_registry.py`

Validate [`tests/benchmarks/fp_registry.toml`](../../../tests/benchmarks/fp_registry.toml) against corpus contracts (`forbid_rules` for `fp` verdicts, `expect_rules` for `tp` verdicts):

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
- [Benchmark calibration](../../design/benchmark-calibration.md)
