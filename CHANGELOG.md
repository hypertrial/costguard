# Changelog

All notable changes to Costguard are documented here. The project follows [Semantic Versioning](https://semver.org/).

## [1.0.0] - 2026-06-12

### Added

- Finding baseline / grandfathering: `--write-baseline`, `--baseline`, and `[output].baseline` in `costguard.toml`.
- SARIF 2.1.0 output format (`--format sarif`) for GitHub Code Scanning, GitLab SAST, and Jenkins.
- SQLCOST027 info diagnostic for SQL parse failures on dbt models.
- `scripts/precision_triage.py` for sampled precision reporting against `fp_registry.toml`.
- External benchmarks: refreshed Dune Spellbook pin, Mattermost warehouse (scan-only), vendored Snowflake snippets.
- Enterprise adoption, GitLab CI, Jenkins, and privacy documentation.

### Changed

- Spellbook benchmark pin updated to `031a5053dd9608ce7e6b9f2d9b16dd9a2fbeba10` (8030 models, 0 parse failures).
- SQLCOST016 no longer flags `date_trunc`/`timestamp_trunc` on partition columns or casts in filters.
- SQLCOST017 demoted to Medium severity after Spellbook precision calibration; added hash and mirror-column join exemptions.
- Path heuristics extended for `core/`, `analytics/`, `mart/`, and `reporting/` folders.
- Benchmark baselines now record `runtime_ms` and enforce `max_runtime_ms` gates.
- jaffle-shop external baseline: 0 parse failures with compiled manifest (was 30% raw failure rate).

### Fixed

- Full-scan mode avoids cloning all SQL documents when filtering scanned files.
- GitHub Action tarball extraction uses `filter="data"`.
- GitHub Action supports `baseline` input for PR checks.

## [0.1.0] - 2026-05-23

- Initial experimental release.
