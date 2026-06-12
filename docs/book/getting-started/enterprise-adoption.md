# Enterprise adoption

Costguard is designed for **baseline-first rollout** in large legacy dbt repos.

## Recommended rollout

1. **Local scan** on a representative branch with compiled manifest:
   ```bash
   costguard scan . --warehouse trino --manifest target/manifest.json --format json
   ```
2. **Write a finding baseline** (grandfather existing debt):
   ```bash
   costguard scan . --warehouse trino --manifest target/manifest.json \
     --write-baseline costguard-baseline.json
   ```
3. **Commit the baseline** to the repo (or store as a CI artifact keyed by branch).
4. **Enable PR checks** with `--baseline costguard-baseline.json --fail-on high`.
5. **Tighten over time** by removing entries from the baseline as teams fix findings.

## CI integration options

| Platform | Format | Notes |
| --- | --- | --- |
| GitHub Actions | `github`, `markdown`, `sarif` | Use the composite Action in `.github/actions/costguard` |
| GitLab CI | `sarif`, `json` | See [GitLab CI](gitlab-ci.md) |
| Jenkins | `sarif`, `json` | See [Jenkins](jenkins.md) |

## Monorepos and Spellbook-style layouts

- Use `dbt-compile-dirs` in the GitHub Action to compile and merge multiple subprojects.
- Pass the merged `target/manifest.json` to Costguard.
- Scan paths can include macros and subproject roots (see Spellbook benchmark config).

## Privacy and data handling

See [Privacy and local-only guarantees](privacy.md). Costguard does not connect to your warehouse or send telemetry.

## Readiness signals

Before enforcing `--fail-on high` repo-wide, validate:

- Model parse failure rate is acceptable (0% compiled failures on Trino Spellbook-scale repos).
- Sampled precision meets team thresholds (see [Benchmark calibration](../../design/benchmark-calibration.md)).
- PR mode scopes to changed files and completes within your CI budget.
