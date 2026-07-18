# Scripts

Helper scripts live under [`scripts/`](../../../scripts/) at the repository root. Prefer `python3` when invoking them.

For Spellbook and other external benchmarks, the script compiles subprojects and **merges** their manifests into `target/manifest.json` at the repo root before scanning. Run `dbt compile` in your own CI job before the Costguard Action; this helper is for benchmarks and local development only.

## `install.sh`

One-liner installer for macOS and Linux release binaries. Downloads a release tarball, verifies SHA256, and installs `costguard` to `/usr/local/bin` or `~/.local/bin`.

```bash
curl -fsSL https://raw.githubusercontent.com/hypertrial/costguard/main/scripts/install.sh | sh
curl -fsSL .../install.sh | sh -s -- v2.6.0
COSTGUARD_INSTALL_DIR="$HOME/.local/bin" curl -fsSL .../install.sh | sh
```

| Env var | Description |
| --- | --- |
| `COSTGUARD_VERSION` | Release tag (default `latest`) |
| `COSTGUARD_INSTALL_DIR` | Install directory (default `/usr/local/bin` or `~/.local/bin`) |
| `COSTGUARD_RELEASE_BASE_URL` | Override download base URL (used by tests) |
| `COSTGUARD_REPO` | GitHub repo slug (default `hypertrial/costguard`) |

See [Installation](../getting-started/installation.md).

## `dbt_compile_for_costguard.py`

Shared dbt compile and manifest merge helper used by `benchmark_external_repo.py` and local Spellbook stress tests. Subproject compiles run in parallel when multiple `--compile-dirs` are provided (`COSTGUARD_DBT_COMPILE_JOBS=1` forces serial). Manifest outputs are cached per repo commit and packages fingerprint when `--cache-dir` is set from the benchmark script.

Benchmark repos configure compile via [`tests/benchmarks/repos.toml`](../../../tests/benchmarks/repos.toml): required external repos compile dbt with dummy offline profiles by default; repo configs may pass `dbt_vars`, `dbt_env`, an existing `dbt_profiles_dir`, and per-repo cache/introspection flags. Tuva compiles its upstream `integration_tests` consumer project with `dbt_preserve_manifest_paths = true` so the scan still targets package source paths. Repos with deterministic offline dbt compile failures use `compile_best_effort = true` to reuse an existing generated or checked-in manifest.

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

Authoritative pre-release qualification gate. It requires the verified signed version tag at `HEAD`, validates the requested workspace version, runs local CI and consumer Action tests, executes pinned external benchmarks, enforces the 10,000-model performance budget, checks external documentation links, and writes `dist/release/release-check.json` bound to the commit. In the release workflow, `--trust-github-qualification` records the already verified `github-ci+benchmark` evidence; `--trust-push-ci` remains a compatibility alias.

```bash
python3 scripts/release_check.py --version 2.0.0
```

`--development`, `--skip-external`, and `--skip-external-links` are development aids. Development mode does not write a release qualification receipt. Strict qualification also requires `mdbook` and `cargo-deny` so documentation and dependency policy checks cannot be silently skipped.

## `verify_ci_history.py`

Release qualification helper used by `release.yml`. `--workflow`, `--event`, and repeatable `--required-job` select the evidence contract. The helper paginates workflow runs and jobs, chooses the latest exact-SHA match, and requires every named job to have completed successfully. Release qualification checks `ci.yml` on `push` (`pr-gate`, which includes synthetic scale) and `benchmark.yml` on `workflow_dispatch` (`full-evidence-gate`, which includes the full local gate and support matrix). Scheduled benchmarks do not qualify.

## `verify_release_assets.py`

Builds a host-platform release tarball using the same layout as [`.github/workflows/release.yml`](../../../.github/workflows/release.yml), verifies its checksum, and smoke-tests the extracted binary. The local release gate runs this before publication.

```bash
python3 scripts/verify_release_assets.py
```

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
  --version 2.0.0 \
  --receipt smoke-x86_64-pc-windows-msvc.json
```

### Packaging recovery checklist

1. Install the cross toolchains from the matrix above.
2. Qualify the exact signed tag locally with `python3 scripts/release_check.py --version 2.0.0`.
3. Build release targets with `python3 scripts/verify_release_assets.py` or `python3 scripts/package_release_target.py`, inspect `SHA256SUMS`, and run native smoke tests as needed.
4. Restore GitHub Actions publication and rerun the immutable tag workflow. Do not upload recovery artifacts manually or replace an exact release.

## `ci_local.sh`

Full local pre-push and release qualification gate. The required `pr-gate` job in [`.github/workflows/ci.yml`](../../../.github/workflows/ci.yml) uses the additive `--fast` subset and then runs `scale_check.py`; the independently dispatched benchmark workflow runs the default full gate before the external support matrix. The script does not itself create release evidence; use `release_check.py` for authoritative release qualification.

```bash
./scripts/ci_local.sh
./scripts/ci_local.sh --fast
./scripts/ci_local.sh --spellbook-smoke
./scripts/ci_local.sh --nba-monte-carlo-smoke
./scripts/ci_local.sh --precision
```

Fast mode runs offline Python lock verification, workspace dependency validation, `ruff check`, Rust fmt/clippy/release build/test, release-asset smoke, and Python unit tests through the lock/Python-fingerprinted `.venv-eval`. The default full mode additionally runs Rustdoc, fp-registry and recall checks, corpus classification metrics, LLM judge IRR validation, vendored benchmarks, generated rule/evidence checks, internal link validation, mdBook, and `cargo deny`.

Unit tests:

```bash
python3 -m unittest discover -s scripts/tests -p 'test_*.py'
```

## `check_docs.py`

Validates repository-local Markdown links during every local CI run. Release qualification adds retrying external URL checks with `--external`.

## `scale_check.py`

Generates independent 2,000-model and 10,000-model clean projects in release mode. Each clean target runs one warmup plus three measured scans. The gate retains the 10,000-model median ≤10 seconds, maximum ≤15 seconds, peak RSS ≤1 GiB, per-model runtime growth ≤1.5×, zero parse failures, and zero diagnostics.

Report version 4 also creates a deterministic 10,000-model Git repository, commits a base project with one known finding, commits a clean model/manifest change that resolves it, then runs one warm-up and two measured `costguard pr --base HEAD~1` replays. It requires the changed file, resolved base finding, processed base context, median ≤30 seconds, maximum ≤45 seconds, and peak RSS ≤1 GiB. The report records all samples and is written even when the gate fails.

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
python3 scripts/benchmark_external_repo.py --repo nba-monte-carlo
python3 scripts/benchmark_external_repo.py --repo nba-monte-carlo --smoke
python3 scripts/benchmark_external_repo.py --repo tuva
python3 scripts/benchmark_external_repo.py --repo ol-data-platform
python3 scripts/benchmark_external_repo.py --repo ol-data-platform --smoke
python3 scripts/benchmark_external_repo.py --repo data-infra  # manual observational

# Refresh baselines after intentional rule tuning
python3 scripts/benchmark_external_repo.py --fixture real_world/jaffle_snippets --update-baseline
python3 scripts/benchmark_external_repo.py --repo spellbook --update-baseline
python3 scripts/benchmark_external_repo.py --repo spellbook --smoke --update-baseline
python3 scripts/benchmark_external_repo.py --repo nba-monte-carlo --update-baseline
python3 scripts/benchmark_external_repo.py --repo nba-monte-carlo --smoke --update-baseline
python3 scripts/benchmark_external_repo.py --repo tuva --update-baseline
python3 scripts/benchmark_external_repo.py --repo ol-data-platform --update-baseline
python3 scripts/benchmark_external_repo.py --repo ol-data-platform --smoke --update-baseline
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
| `--cost` | Include cost summary in benchmark JSON reports (also enabled per-repo via `cost = true` in `repos.toml`) |
| `--warehouse` | Override scan warehouse (defaults per target) |

Committed Grade C cost priors for external repos live in `tests/benchmarks/cost-configs/{repo}.toml`. When cost is enabled, the harness copies that file into the checkout as `costguard.toml` before scanning. Dollar figures are estimates only; see [Cost estimates](cost-estimates.md#benchmark-repo-cost-configs).

Reports include `compile_cache: hit|miss|skip` when dbt compile is enabled. External reports may include a `cost` block when cost estimation is enabled. Benchmark scripts use release CLI builds via `costguard_tooling.py`.

Repos may set `dbt_compile_shim` in [`tests/benchmarks/repos.toml`](../../../tests/benchmarks/repos.toml) to copy an offline macro override into `dbt_packages/trino_utils` after `dbt deps` (used by `ol-data-platform` for `get_intervals_between` during DuckDB compile).

Validate vendored baselines in Rust:

```bash
cargo test -p costguard-core --test benchmark vendored_baselines_match
```

### `audit-compiled-parse` binary

Not installed by `cargo install --path crates/costguard-cli`. Build explicitly:

```bash
cargo run -p costguard-sql --bin audit-compiled-parse --features audit-bin -- path/to/manifest.json --bucket
cargo run -p costguard-sql --bin audit-compiled-parse --features audit-bin -- path/to/manifest.json --json
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

Bucket per-file diagnostics for external-repo triage after running the Spellbook benchmark. **Nineteen rules** have dedicated regex/AST classifiers (`SQLCOST002`, `003`, `005`, `006`, `008`, `012`–`020`); all other rules fall through to bucket `other`. See [Manual rule review playbook](../../design/manual-rule-review.md#bucket-classifiers).

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

## `rule_tp_census.py`

Cost-ranked FP-elimination census across benchmark repos. For each behavioral rule, examines up to **100** findings (100% if fewer), ranked by `cost_estimate.savings_p50_usd_per_month`. **PASS** when the examined sample has **0 `fp_bug` and 0 `unknown`** (`tp` and documented `exempt` findings are both acceptable). Infrastructure rules SQLCOST023–027 and SQLCOST045–047 auto-pass.

```bash
python3 scripts/rule_tp_census.py --emit-evidence
python3 scripts/rule_tp_census.py --repos spellbook data-infra --json
python3 scripts/rule_tp_census.py --rule SQLCOST012 --sample-cap 50
python3 scripts/rule_tp_census.py --force-compile --emit-evidence
python3 scripts/rule_tp_census.py --emit-stratified-evidence
```

| Flag | Description |
| --- | --- |
| `--repos` | Repo names from `repos.toml` (default: all four) |
| `--cache` | Benchmark cache root |
| `--rule` | Census a single rule id (fast iteration) |
| `--sample-cap` | Max findings examined per rule (default: 100) |
| `--force-compile` | Bypass cached dbt manifest |
| `--json` | Emit JSON report to stdout |
| `--emit-evidence` | Write `tests/benchmarks/rule_tp_evidence.json` |
| `--emit-stratified-evidence [path]` | Write supplemental tail evidence grouped by repo and bucket after the primary cost-ranked sample |
| `--supplemental-rules` | Rule ids for supplemental evidence (default: high-volume review targets plus `SQLCOST027`) |
| `--supplemental-per-bucket-cap` | Max supplemental findings per repo/bucket (default: 10) |

Exit code `1` when any rule fails the pass bar. See [Rule TP coverage](../../design/rule-tp-coverage.md) and [Manual rule review](../../design/manual-rule-review.md).

## `top_findings_review.py`

Rank top-N cost findings with SQL context, bucket, and registry verdict. Used for cost-prioritized Spellbook triage loops.

```bash
python3 scripts/top_findings_review.py --repo spellbook --top 50
python3 scripts/top_findings_review.py --repo spellbook --rule SQLCOST014 --top 20
python3 scripts/top_findings_review.py --repo spellbook --top 10 --json
```

| Flag | Description |
| --- | --- |
| `--repo` | Repo name from `repos.toml` (default `spellbook`) |
| `--top` | Number of findings to rank (default `10`) |
| `--rule` | Filter to one rule id |
| `--context` | SQL context lines around finding (default `12`) |
| `--json` | Emit JSON instead of text |

Requires `--cost` scan (enabled automatically). Rank key: `savings_p50_usd_per_month`, then `relative_index`.

## `build_eval_dataset.py`

Seed or refresh the frozen binary-classification dataset at [`tests/benchmarks/eval_labels.toml`](../../../tests/benchmarks/eval_labels.toml).

```bash
python3 scripts/build_eval_dataset.py --write
python3 scripts/build_eval_dataset.py --write --sample-negatives 200 --negative-repo spellbook
```

| Flag | Description |
| --- | --- |
| `--write` | Write `eval_labels.toml` |
| `--out` | Output path (default `tests/benchmarks/eval_labels.toml`) |
| `--sample-negatives` | Sample non-fired `(path, rule)` pairs from cached external repo |
| `--negative-seed` | RNG seed for negative sampling |
| `--negative-repo` | External repo for negative sampling (default `spellbook`) |

## `eval_metrics.py`

Compute binary-classification metrics (precision, recall, F1, MCC, balanced accuracy, PR-AUC, ROC-AUC, Wilson CIs) from the frozen label set. Requires eval dependencies:

```bash
python3 -m venv .venv-eval
.venv-eval/bin/pip install --require-hashes -r requirements-eval.lock
.venv-eval/bin/python scripts/eval_metrics.py --split corpus
.venv-eval/bin/python scripts/eval_metrics.py --split real
.venv-eval/bin/python scripts/eval_metrics.py --split all --json-out triage/eval.json
```

| Flag | Description |
| --- | --- |
| `--labels` | Label file (default `tests/benchmarks/eval_labels.toml`) |
| `--split` | `corpus`, `real`, or `all` |
| `--cache` | External benchmark cache root |
| `--json-out` | Write JSON report |

The corpus split is hard-gated in `./scripts/ci_local.sh`. The real split runs behind `--precision`. See [Classification metrics](../../design/classification-metrics.md).

## `build_llm_judge_labels.py`

Local-only tool that runs a pinned Qwen3-30B-A3B GGUF judge (via llama-cpp-python + Metal) over spellbook findings and writes committed inter-rater labels. **Not run in CI.**

Requires a local GGUF and the hashed `requirements-judge.lock`:

```bash
python3 -m venv .venv-judge
.venv-judge/bin/pip install --require-hashes -r requirements-judge.lock
export COSTGUARD_JUDGE_GGUF=/path/to/model.gguf
.venv-judge/bin/python scripts/build_llm_judge_labels.py --model "$COSTGUARD_JUDGE_GGUF"
python3 scripts/build_llm_judge_labels.py --dry-run
```

| Flag | Description |
| --- | --- |
| `--repo` | External repo (default `spellbook`) |
| `--model` | Path to local GGUF (or set `COSTGUARD_JUDGE_GGUF`) |
| `--cap` | Max findings per `(rule, bucket)` (default 50) |
| `--seed` | Deterministic sampling seed (default 3407) |
| `--context-tokens` | llama.cpp context size (default 32768) |
| `--n-batch` | llama.cpp batch size (default 2048) |
| `--n-ubatch` | llama.cpp micro-batch size (default 512) |
| `--sql-token-target` | Per-file SQL excerpt target in tokens (default 8000) |
| `--rule-id` | Limit to rule ID(s); repeatable (e.g. `--rule-id SQLCOST012`) |
| `--grouped` | One LLM call per file with JSON verdict array (faster) |
| `--checkpoint-every` | Write labels JSONL after every N files (default 1) |
| `--dry-run` | Enumerate capped candidates without loading the model |
| `--out` | Output JSONL (default `tests/benchmarks/llm_judge_labels.jsonl`) |
| `--manifest-out` | Manifest TOML (default `tests/benchmarks/llm_judge_manifest.toml`) |

See [LLM judge IRR](../../design/llm-judge-irr.md).

## `lock_python_deps.py`

Regenerate both Python locks with maintainer-installed `uv`, or verify their embedded direct-input SHA-256 metadata without network access or `uv`:

```bash
python3 scripts/lock_python_deps.py
python3 scripts/lock_python_deps.py --check
```

## `eval_irr.py`

Validate committed LLM judge labels against the pinned manifest and recompute Cohen's κ, MCC, and class recall/precision (report-only; no κ floor gate). Runs in `./scripts/ci_local.sh` via `.venv-eval`:

```bash
.venv-eval/bin/python scripts/eval_irr.py
.venv-eval/bin/python scripts/eval_irr.py --json-out tests/benchmarks/irr_report.json
```

| Flag | Description |
| --- | --- |
| `--labels` | Judge JSONL (default `tests/benchmarks/llm_judge_labels.jsonl`) |
| `--manifest` | Judge manifest (default `tests/benchmarks/llm_judge_manifest.toml`) |
| `--json-out` | Output report (default `tests/benchmarks/irr_report.json`) |

## `precision_triage.py`

Sample external-repo findings and compute precision against [`fp_registry.toml`](../../../tests/benchmarks/fp_registry.toml) bucket verdicts. Used for Spellbook governance readiness gates (≥90% high, ≥80% overall).

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

Validate corpus **coverage** for behavioral rules (SQLCOST001–022 and SQLCOST028–044): at least two `expect_rules` cases and one `forbid_rules` case per rule in [`tests/fixtures/corpus/manifest.toml`](../../../tests/fixtures/corpus/manifest.toml). This is not operational recall; use `eval_metrics.py` for precision/recall/MCC/F1.

```bash
python3 scripts/recall_report.py
python3 scripts/recall_report.py --rules SQLCOST030 SQLCOST031
```

Exit code `1` when any checked rule falls below the minimum case counts.

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

- [Manual rule review playbook](../../design/manual-rule-review.md)
- [Benchmark tiers](../contributing/benchmark-tiers.md)
- [Benchmark calibration](../../design/benchmark-calibration.md)
