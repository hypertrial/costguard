# Quick start (PR check)

Automated PR review is Costguard's primary workflow. The local CLI powers GitHub Actions, local debugging, and CI.

For a zero-config local scan with no flags or config file, see [Local scan and explain](local-scan.md). Install with [Installation](installation.md); scaffold CI with `costguard init`.

## PR command

```bash
costguard pr --base origin/main --warehouse snowflake --fail-on high --min-confidence high --block-only-new
```

| Flag | Notes |
| --- | --- |
| `--base` | Git ref to diff against. CLI default is `main`; use `origin/main` in CI after checkout with history. |
| `--warehouse` | SQL dialect for parsing heuristics. See [Platforms](../reference/platforms.md). |
| `--fail-on` | Minimum severity that fails the run. Default when unset in config: `high`. |
| `--min-confidence` | Optional confidence floor for fail logic. Recommended for PR gates: `high` (suppresses regex-only shape hits on Jinja-heavy models). |
| `--block-only-new[=true\|false]` | Enforce severity and addressable-savings gates only for introduced or regressed findings. Bare flag means `true`; CLI/config default remains `false`. |
| `--fail-on-pr-cost-increase` | Optional priced project-wide net-cost threshold in USD/month. Requires `[cost.pricing].model`. |

## GitHub Action

Use the published composite action after your existing dbt compile step:

```yaml
permissions:
  contents: read
  pull-requests: write

jobs:
  costguard:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v6
        with:
          fetch-depth: 0
      - run: dbt compile --target dev
      - uses: hypertrial/costguard/.github/actions/costguard@v2.7.0
        with:
          base: origin/main
          warehouse: snowflake
          manifest: target/manifest.json
          fail-on: high
          min-confidence: high
          block-only-new: true
          format: github
          receipt-path: costguard-receipt.json
          pr-comment: true
          github-token: ${{ github.token }}
```

For Costguard contributor workflows that need to run the checked-out source instead of a release binary, add:

```yaml
    install-mode: source
```

Core inputs: `base`, `warehouse`, `manifest`, `fail-on`, and `baseline`. The Action defaults `block-only-new` to `true` and always forwards the explicit value; set it to `false` during an upgrade if all-findings enforcement is still required. It also supports `min-confidence`, `format`, `analysis-policy`, `fail-on-cost-delta`, `fail-on-pr-cost-increase`, signed-policy inputs, `receipt-path`, and `compare-receipt`. It always writes markdown to the GitHub step summary while preserving the selected stdout format.

Sticky PR comments are opt-in for existing Action consumers: set `pr-comment: true`, pass `github-token: ${{ github.token }}`, and grant `pull-requests: write`. The Action creates or updates one marker-based comment using the same Markdown as the step summary. Fork and Dependabot PRs commonly receive a read-only token; comment publication then warns and skips without changing Costguard's scan result. Newly generated `costguard init` workflows include this setup.

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

The Action does not install or compile dbt or Rocky. See [Requirements](requirements.md) for manifest, git history, and compile guidance, and [Rocky integration](rocky.md) for sealed head/base artifact workflows.

Pair `fail-on: high` with `min-confidence: high` on macro-heavy dbt projects so PR gates keep AST-confirmed findings and ignore regex-only noise (for example SQLCOST012 comma joins detected without a successful parse).

For a calibrated project-wide dollar guardrail, configure priced cost inputs and add `fail-on-pr-cost-increase: "1000"`. This gate uses `pr_impact.net.monthly_p50`; avoided or negative net cost passes. `fail-on-cost-delta` remains a separate addressable finding-savings gate.

The Action defaults to `analysis-policy: standard`. Set `analysis-policy: strict` in the Action or committed `costguard.toml` when git-native governance requires manifest-backed analysis. Manifest behavior: [Requirements](requirements.md).

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

For non-Action CI, combine `--format github --summary-file summary.md --receipt-file receipt.json` to produce annotations, a human summary, and a machine receipt from one scan. Pass `--compare-receipt` on a later run to record trend deltas.

See [Output formats](../reference/output.md) for the JSON schema.

## Manifest and compiled SQL

For Jinja-heavy dbt models, run `dbt compile` in your existing CI job before Costguard. Costguard loads `compiled_code` from the manifest for parse metrics. When a finding is only available from compiled SQL and cannot be mapped back to raw source, JSON output marks it as `source_provenance: "compiled_unmapped"` and includes the compiled line and column.

See [Requirements](requirements.md) for the full manifest contract.

## Related

- [PR check primary workflow](../../design/pr-check-primary-workflow.md)
- [Enterprise adoption](enterprise-adoption.md)
- [Suppressions](../reference/suppressions.md)
