# Changelog

All notable changes to Costguard are documented here. The project follows [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- Binary-classification evaluation pipeline: frozen [`tests/benchmarks/eval_labels.toml`](tests/benchmarks/eval_labels.toml), [`scripts/build_eval_dataset.py`](scripts/build_eval_dataset.py), and [`scripts/eval_metrics.py`](scripts/eval_metrics.py) computing precision/recall/F1/MCC/PR-AUC/ROC-AUC with Wilson CIs (eval deps in [`requirements-eval.txt`](requirements-eval.txt)).
- LLM-as-judge inter-rater reliability: [`scripts/build_llm_judge_labels.py`](scripts/build_llm_judge_labels.py) (local-only, llama-cpp-python), [`scripts/eval_irr.py`](scripts/eval_irr.py) (CI-safe κ validation), committed [`tests/benchmarks/llm_judge_labels.jsonl`](tests/benchmarks/llm_judge_labels.jsonl).
- Design doc: [Classification metrics](docs/design/classification-metrics.md).
- Design doc: [LLM judge IRR](docs/design/llm-judge-irr.md).
- `scripts/install.sh` — one-liner release installer for macOS and Linux (`curl | sh`).
- `costguard init` — scaffolds `.github/workflows/costguard.yml` and starter `costguard.toml` with best-effort warehouse detection.
- Documentation: [Installation](docs/book/getting-started/installation.md) and [Requirements](docs/book/getting-started/requirements.md); README restructured for install → run → CI onboarding.

### Changed

- LLM judge v3: structured JSON verdict (`exemption_applies`, `failure_condition_met`, `verdict`) with balanced rubric prompt, per-rule few-shots (`tests/benchmarks/judge_fewshots.toml`), deterministic `map_structured_verdict`, per-file checkpointing, and richer IRR metrics (MCC + registry FP/TP recall/precision). Compact GBNF grammar required for llama.cpp (spaced `ws` grammar stalls generation).
- LLM judge v2: ChatML `create_chat_completion` (fixes instruct-model confirm-bias), anti-FP prompt, `n_ctx=32768`, `sql_token_target=8000`, model-chosen `C` abstention (drops logprob margin), `--rule-id` filter, `flash_attn=true`.
- LLM judge build tool: per-file SQL context packing with KV prefix reuse (default), optional `--grouped` one-call-per-file mode, and tunable batch/SQL token targets for M4 Air performance.
- Folded `costguard-git` into `costguard-core` and `costguard-platform` into `costguard-sql` (11 workspace crates).

### Removed

- Orphaned scripts: `audit_compiled_parse_failures.py`, `calibrate_cost_model.py`, `configure_github_release.py`, and `publish_release_local.py`. Use `cargo run -p costguard-sql --bin audit-compiled-parse` for manifest parse audits.
- Unused protocol type `FindingV1`, scanner wrappers, and dead evidence helpers.

### Fixed

- `Estimate::Sub` now computes a dollar difference instead of a log-ratio, so `potential_savings` reports real savings instead of a mis-scaled value.
- Project `post_fix_cost` and `potential_savings` are deduplicated per model (not per finding), so post-fix cost cannot exceed current cost on repos with many findings per model.

## [2.2.0] - 2026-06-15

### Added

- `[cost.inputs].observations`, `observations_before`, and `observations_after` for normalized warehouse observation bundles (Grade-A input).
- Four-number cost reporting: `current_cost`, `post_fix_cost`, `potential_savings`, plus optional `pr_impact` and `realized_savings` with monthly/annual values, uncertainty, coverage, and estimation basis.
- Default multipliers for SQLCOST036–SQLCOST044; cost-bearing rules emit either an estimate or an explicit `unestimated_reason`.
- `costguard-git::file_at_ref` for base-branch PR cost comparison.
- JSON output `schema_version: 4` with structured `cost` blocks (`CostFigure`, `CoverageMetrics`, `PrCostImpact`, `RealizedSavings`).
- Per-finding `current_cost_p50_usd_per_month`, `post_fix_cost_p50_usd_per_month`, and `unestimated_reason` on `CostEstimate`.

### Changed

- Savings counterfactual uses `1 - 1/multiplier` (not `multiplier - 1`); per-model caps are multiplicative. Recalibrate `fail_on_monthly_delta` thresholds if tuned under the old formula.
- Measured observations and query-history bytes skip partition/view/incremental priors.
- `fail_on_monthly_delta` prefers `pr_impact.net` when PR mode produces a base/head comparison.

### Removed

- Migration CLI (`baseline identity-map-v2`, `baseline migrate-v2`, `baseline migrate-v1`, `policy migrate-v1`) and ordinal identity-map machinery after 2.1 rollout completion.

## [2.1.0] - 2026-06-14

### Added

- Semantic finding identity (`semantic-v1`) replaces ordinal `occurrence:N` evidence keys; finding IDs survive formatting and line movement.
- Unified diagnostic pipeline with ordered governance stages (overrides, suppressions, identity, policy, baseline).
- Baseline schema v3 and policy schema v2, both requiring `identity_scheme = "semantic-v1"`.
- Migration commands: `costguard baseline identity-map-v2`, `costguard baseline migrate-v2`, `costguard policy migrate-v1`.
- PR target/context separation: changed files are gated; unchanged parse failures appear in optional `context` reporting only.
- Optional top-level `identity_scheme` field in JSON output schema v3.
- Canonical 44-rule built-in catalog in `costguard-protocol`.

### Changed

- **Breaking:** baseline v2 and policy v1 are rejected at scan time with exact migration-command guidance.
- `costguard explain` ignores configured severity and cost gates; exits 0 when analysis completes.
- `baseline migrate-v1` now emits baseline v3 with semantic identities.
- Duplicate semantic findings collapse to one diagnostic; legacy ordinal aliases captured in identity maps.

### Migration

Deploy the 2.1 binary, policy v2, and baseline v3 atomically. Retain policy v1 and baseline v2 artifacts for rollback to 2.0.

## [2.0.0] - 2026-06-14

### Added

- Stable local CLI and GitHub Action for dbt-aware SQL cost regression checks across Generic SQL, Snowflake, BigQuery, and Trino.
- Signed policy enforcement, JSON schema v3, SARIF output, baselines, and advisory local cost estimates.
- Reproducible four-platform release packages with checksums, SBOMs, and provenance attestations.

### Changed

- Promoted the tested RC2 runtime to the MVP production release without runtime or schema changes.
- Simplified release qualification to one successful exact-SHA push CI run while retaining local CI, scale, signed-tag, packaging, and consumer smoke gates.

### Known limitations

- Databricks, Redshift, Postgres, and DuckDB analysis remains preview.
- Cost estimates are advisory and require locally supplied statistics or offline exports; Costguard does not connect to warehouses.
- The GitHub Action expects callers to compile dbt when manifest-backed analysis is desired.

## [2.0.0-rc.2] - 2026-06-13

### Changed

- Replaced quadratic manifest model lookup and full-scan document filtering with indexed, linear-time paths.
- Strengthened the 10,000-model release gate with a 2,000-model scaling baseline, nonlinear-growth detection, and version 3 evidence reports that are written on failure.

### Fixed

- Restored the 10,000-model scan to the release performance budget on macOS development hardware.

## [2.0.0-rc.1] - 2026-06-13

### Added

- **Model-centric cost engine (v2):** per-model monthly cost index, deduplicated project totals, savings attribution with per-model caps, structure factors from SQL features, and lineage fan-out weighting.
- `ProjectCostSummary` on scan results; JSON schema version 3 with top-level `cost` block.
- `costguard cost` subcommand for local cost prioritization summaries without requiring findings.
- `--cost` on `explain`; `fail_on_monthly_delta_gb` for GB-month savings gating without pricing.
- New `CostEstimate` fields: `model_id`, `model_monthly_p50_usd`, `savings_p*_usd_per_month`.

### Changed

- **Breaking (cost semantics):** finding `p50_usd_per_month` now represents estimated **savings**, not total inflated model×rule cost. Use `model_monthly_p50_usd` for model baseline spend.
- `--fail-on-cost-delta` gates on deduplicated savings sum; failure messages print computed total vs threshold.
- Catalog row counts used as grade-B volume fallback; symmetric catalog key matching; quoted-field CSV parsing for query history.
- Incremental byte discount preserves lognormal σ; skipped for unbounded-incremental rules (SQLCOST004/005/019/029).

## Internal development milestone: cost estimates - 2026-06-12

### Added

- Per-finding cost estimates with lognormal p10/p50/p90 intervals and provenance grades (A/B/C).
- `[cost]` section in `costguard.toml` for pricing, catalog/query-history inputs, and multiplier overrides.
- `costguard-cost` crate: volume resolver, rule multiplier priors, scan/compute pricing regimes.
- CLI flags `--cost` and `--fail-on-cost-delta`; GitHub Action inputs `cost` and `fail-on-cost-delta`.
- `scripts/calibrate_cost_model.py` for offline query-history calibration and coverage reporting.
- Cost estimate fixture and integration tests; documentation at `docs/book/reference/cost-estimates.md`.

### Changed

- README rule count updated to 35; diagnostics include optional `cost_estimate` in JSON/SARIF output.
- Text/markdown output ranks top findings by estimated monthly cost when cost is enabled.

## Internal development milestone: baseline and SARIF - 2026-06-12

### Added

- Finding baseline / grandfathering: `--write-baseline`, `--baseline`, and `[output].baseline` in `costguard.toml`.
- SARIF 2.1.0 output format (`--format sarif`) for GitHub Code Scanning, GitLab SAST, and Jenkins.
- SQLCOST027 info diagnostic for SQL parse failures on dbt models.
- SQLCOST028–SQLCOST035: mart partition/cluster config, incremental full-refresh config, correlated subqueries, leading-wildcard LIKE, OR partition predicates, pattern-matching joins, scalar subqueries in SELECT, and cross-catalog joins.
- `scripts/precision_triage.py` for sampled precision reporting against `fp_registry.toml`.
- `scripts/recall_report.py` CI gate for minimum corpus recall coverage (≥2 `expect_rules`, ≥1 `forbid_rules` per behavioral rule).
- Recall corpus fixtures for SQLCOST001–022 and SQLCOST028–035 with platform-aware corpus harness support.

### Changed

- SQLCOST008 skips queries containing `GROUP BY` (dedupe via aggregation is treated as intentional).
- SQLCOST012 demoted to Medium severity after Spellbook precision calibration.
- SQLCOST014 requires a CTE to be referenced three or more times before firing.
- SQLCOST015 requires an expensive expression in three or more files before firing.
- Spellbook benchmark pin updated to `031a5053dd9608ce7e6b9f2d9b16dd9a2fbeba10` (8030 models, 0 parse failures).
- SQLCOST016 no longer flags `date_trunc`/`timestamp_trunc` on partition columns or casts in filters.
- SQLCOST017 demoted to Medium severity after Spellbook precision calibration; added hash and mirror-column join exemptions.
- Path heuristics extended for `core/`, `analytics/`, `mart/`, and `reporting/` folders.
- Benchmark reports record runtime samples, median, maximum, and peak RSS using internal schema version 2.
- jaffle-shop external baseline: 0 parse failures with compiled manifest (was 30% raw failure rate).
- External benchmarks: refreshed Dune Spellbook pin, Mattermost warehouse (scan-only), vendored Snowflake snippets.
- Enterprise adoption, GitLab CI, Jenkins, and privacy documentation.

### Fixed

- SQLCOST006 no longer fires when the join predicate text contains `=`.
- Full-scan mode avoids cloning all SQL documents when filtering scanned files.
- GitHub Action tarball extraction uses `filter="data"`.
- GitHub Action supports `baseline` input for PR checks.

## [0.1.0] - 2026-05-23

- Initial experimental release.

`v0.1.0` was the only public release before the `2.x` line. The internal milestones above were never published as releases.
