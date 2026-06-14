# Quick start (PR check)

Automated PR review is Costguard's primary workflow. The local CLI powers GitHub Actions, local debugging, and CI.

For a zero-config local scan with no flags or config file, see [Local scan and explain](local-scan.md).

## MVP command

```bash
costguard pr --base origin/main --warehouse snowflake --fail-on high --min-confidence high
```

| Flag | Notes |
| --- | --- |
| `--base` | Git ref to diff against. CLI default is `main`; use `origin/main` in CI after checkout with history. |
| `--warehouse` | SQL dialect for parsing heuristics. See [Platforms](../reference/platforms.md). |
| `--fail-on` | Minimum severity that fails the run. Default when unset in config: `high`. |
| `--min-confidence` | Optional confidence floor for fail logic. Recommended for PR gates: `high` (suppresses regex-only shape hits on Jinja-heavy models). |

## GitHub Action

Use the published composite action after your existing dbt compile step:

```yaml
- uses: actions/checkout@v6
  with:
    fetch-depth: 0
- run: dbt compile --target dev
- uses: hypertrial/costguard/.github/actions/costguard@v2.1.0
  with:
    base: origin/main
    warehouse: snowflake
    manifest: target/manifest.json
    fail-on: high
    min-confidence: high
    format: github
```

For Costguard contributor workflows that need to run the checked-out source instead of a release binary, add:

```yaml
    install-mode: source
```

Core inputs: `base`, `warehouse`, `manifest`, `fail-on`, and `baseline`. The Action also supports `min-confidence`, `format`, `analysis-policy`, optional cost flags, and signed-policy inputs. Pin `@v2.1.0` for immutable behavior or use `@v2` for compatible stable updates.

Costguard 2.1 requires baseline v3 and policy v2 with `identity_scheme: "semantic-v1"`. See [Compatibility policy](../reference/compatibility.md).

Enterprise strict mode passes only configured governance values:

```yaml
    analysis-policy: strict
    policy: .costguard/policy.signed.json
    trust-store: .costguard/trust.json
    policy-organization: acme
    policy-team: data-platform
    policy-repository: acme/warehouse
```

The Action does not install or compile dbt. Run `dbt compile` in a prior step so `target/manifest.json` is available when you want manifest-backed analysis.

Pair `fail-on: high` with `min-confidence: high` on macro-heavy dbt projects so PR gates keep AST-confirmed findings and ignore regex-only noise (for example SQLCOST012 comma joins detected without a successful parse).

The Action defaults to `analysis-policy: standard`. Costguard auto-detects `target/manifest.json` when present; raw analysis still works without a manifest. Set `analysis-policy: strict` in the Action or committed `costguard.toml` when git-native governance requires manifest-backed analysis.

For dbt Cloud or private-package workflows, let your existing authenticated dbt job produce a manifest artifact, download it before this action, and set `manifest: target/manifest.json`. Costguard does not require warehouse credentials.

## CI output formats

```bash
costguard pr --base origin/main --warehouse snowflake --fail-on high --min-confidence high --format github
costguard pr --base origin/main --warehouse snowflake --fail-on high --min-confidence high --format markdown
costguard pr --base origin/main --warehouse snowflake --fail-on high --min-confidence high --format json
```

- `github` — annotation commands for GitHub Checks
- `markdown` — PR-summary-oriented report
- `json` — structured `diagnostics` and optional `pr_summary`
- `sarif` — retained by your CI platform for audit and triage

See [Output formats](../reference/output.md) for the JSON schema.

## Manifest and compiled SQL

For Jinja-heavy dbt models, run `dbt compile` in your existing CI job before Costguard. Costguard loads `compiled_code` from the manifest for parse metrics. When a finding is only available from compiled SQL and cannot be mapped back to raw source, JSON output marks it as `source_provenance: "compiled_unmapped"` and includes the compiled line and column.

If `--manifest` is omitted, Costguard auto-loads `target/manifest.json` when that file exists in the scan root.

## Related

- [PR check primary workflow](../../design/pr-check-primary-workflow.md)
- [Enterprise adoption](enterprise-adoption.md)
- [Suppressions](../reference/suppressions.md)
