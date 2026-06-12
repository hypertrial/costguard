# Changelog

All notable changes to Costguard are documented here. The project follows [Semantic Versioning](https://semver.org/).

## [1.1.0] - 2026-06-12

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

## [1.0.0] - 2026-06-12

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
- Benchmark baselines now record `runtime_ms` and enforce `max_runtime_ms` gates.
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
