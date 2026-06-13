# Enterprise adoption

Costguard is a local, dbt-aware cost regression guardrail for git workflows. Enterprise adoption uses the same binary with governance implemented through git.

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
3. **Commit the baseline** to the repo. Protect it with CODEOWNERS and branch rules.
4. **Enable PR checks** with `--baseline costguard-baseline.json --fail-on high --min-confidence high`.
5. **Tighten over time** by removing entries from the baseline as teams fix findings.
6. **Add signed policy and exceptions** when teams need centralized enforcement. See [Signed policy and exceptions](../governance/policy.md).

## CI integration options

| Platform | Format | Notes |
| --- | --- | --- |
| GitHub Actions | `github`, `markdown`, `sarif` | Use the composite Action in `.github/actions/costguard` |
| GitLab CI | `sarif`, `json` | See [GitLab CI](gitlab-ci.md) |
| Jenkins | `sarif`, `json` | See [Jenkins](jenkins.md) |

Run `dbt compile` in your existing CI job before Costguard. The Action auto-detects `target/manifest.json` when present.

## Organization-level workflows

Publish a reusable workflow or shared pipeline template that pins the Costguard release version, sets `fail-on: high` and `min-confidence: high`, and passes a committed baseline path. Retain JSON and SARIF artifacts in your existing CI platform for audit and triage.

Use the [enterprise operations reference](enterprise-operations.md) for strict signed-policy Action configuration, baseline rollout, key rotation, exception lifecycle, SARIF retention, and rollback.

## Scheduled full-repository scans

Keep PR checks changed-file scoped. Add a scheduled job (nightly or weekly) that runs `costguard scan` on the default branch with `--format sarif` or `--format json` for repo hygiene and drift detection.

## Monorepos and Spellbook-style layouts

- Compile each dbt subproject in your existing CI job and merge manifests before Costguard runs.
- Pass the merged `target/manifest.json` with `--manifest`.
- Scan paths can include macros and subproject roots (see Spellbook benchmark config).

## Privacy and data handling

See [Privacy and local-only guarantees](privacy.md). Costguard does not connect to your warehouse or send telemetry.

## Readiness signals

Before enforcing `--fail-on high` repo-wide, validate:

- Model parse failure rate is acceptable (0% compiled failures on Trino Spellbook-scale repos).
- Sampled precision meets team thresholds (see [Benchmark calibration](../../design/benchmark-calibration.md)).
- PR mode scopes to changed files and completes within your CI budget.
