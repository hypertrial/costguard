# Quick start (PR check)

Automated PR review is Costguard's primary workflow. The local CLI powers GitHub Actions, local debugging, and CI.

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

Use the published composite action:

```yaml
- uses: actions/checkout@v4
  with:
    fetch-depth: 0
- uses: hypertrial/costguard/.github/actions/costguard@v1
  with:
    base: origin/main
    warehouse: snowflake
    fail-on: high
    min-confidence: high
    format: github
```

For Costguard contributor workflows that need to run the checked-out source instead of a release binary, add:

```yaml
    install-mode: source
```

Inputs: `base`, `warehouse`, `fail-on`, `min-confidence`, `format` (`github` \| `markdown` \| `json` \| `text`), optional `manifest`, `working-directory`, optional `cost` and `fail-on-cost-delta`, release install settings (`install-mode`, `version`, `verify-attestation`), analysis settings (`analysis-policy`), and dbt compile settings (`compile-dbt`, `dbt-installation`, `dbt-target`, `dbt-project-dir`, `dbt-profiles-dir`, `dbt-adapter-package`, `dbt-profile-type`, `dbt-compile-dirs`, `manifest-output`, `dbt-requirements-file`, `dbt-constraints-file`, `dbt-vars`, `fail-on-deps-failure`, `use-existing-manifest`, `allow-credentialed-compile`). Use `@v1` for compatible updates or `@v1.1.0` for an immutable pin.

Pair `fail-on: high` with `min-confidence: high` on macro-heavy dbt projects so PR gates keep AST-confirmed findings and ignore regex-only noise (for example SQLCOST012 comma joins detected without a successful parse).

The Action defaults to `analysis-policy: strict`, which requires a dbt manifest for dbt projects and fails closed on parse or metadata gaps. Set `analysis-policy: standard` for best-effort analysis when a manifest is unavailable.

When `compile-dbt: true`, the action runs the shared [`dbt_compile_for_costguard.py`](../../../scripts/dbt_compile_for_costguard.py) helper (same logic as the Spellbook benchmark harness): `dbt deps`, `dbt compile`, optional multi-subproject manifest merge, then passes `--manifest` when present. Compile uses a dummy local profile by default (no warehouse connection). Set `allow-credentialed-compile: true` only when you explicitly configure a real profiles directory. Use `dbt-installation: locked` with a hash-locked `dbt-requirements-file` for reproducible adapter installs. The adapter package is derived from `warehouse`; `generic` requires an explicit `dbt-adapter-package`. Set `dbt-profile-type` only when the package name does not identify the profile type.

Enterprise dbt repos should pin dbt dependencies with `dbt-requirements-file` or `dbt-constraints-file`, pass required compile variables through `dbt-vars`, and set `fail-on-deps-failure: true` when package resolution must be enforced. If another workflow already uploads `target/manifest.json`, set `use-existing-manifest: true` and provide `manifest` or `manifest-output` to run Costguard in artifact-only mode.

For monorepos with multiple dbt subprojects (Spellbook-style), set `dbt-compile-dirs` to a comma- or newline-separated list and `manifest-output` to the merged root manifest path (default `target/manifest.json`).

For dbt Cloud or private-package workflows, let your existing authenticated dbt job produce a manifest artifact, download it before this action, and run with `use-existing-manifest: true`. Costguard does not require warehouse credentials.

## CI output formats

```bash
costguard pr --base origin/main --warehouse snowflake --fail-on high --min-confidence high --format github
costguard pr --base origin/main --warehouse snowflake --fail-on high --min-confidence high --format markdown
costguard pr --base origin/main --warehouse snowflake --fail-on high --min-confidence high --format json
```

- `github` — annotation commands for GitHub Checks
- `markdown` — PR-summary-oriented report
- `json` — structured `diagnostics` and optional `pr_summary`

See [Output formats](../reference/output.md) for the JSON schema.

## Manifest and compiled SQL

For Jinja-heavy dbt models, run `dbt compile` first (or enable compile in the Action). Costguard loads `compiled_code` from the manifest for parse metrics. When a finding is only available from compiled SQL and cannot be mapped back to raw source, JSON output marks it as `source_provenance: "compiled_unmapped"` and includes the compiled line and column.

If `--manifest` is omitted, Costguard auto-loads `target/manifest.json` when that file exists in the scan root.

## Related

- [PR check primary workflow](../../design/pr-check-primary-workflow.md)
- [Suppressions](../reference/suppressions.md)
