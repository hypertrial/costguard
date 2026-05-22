# Quick start (PR check)

Automated PR review is Costguard's primary workflow. The local CLI powers GitHub Actions, local debugging, and CI.

## MVP command

```bash
costguard pr --base origin/main --warehouse snowflake --fail-on high
```

| Flag | Notes |
| --- | --- |
| `--base` | Git ref to diff against. CLI default is `main`; use `origin/main` in CI after checkout with history. |
| `--warehouse` | SQL dialect for parsing heuristics. See [Platforms](reference/platforms.md). |
| `--fail-on` | Minimum severity that fails the run. Default when unset in config: `high`. |

## GitHub Action

Use the composite action at [`.github/actions/costguard`](https://github.com/hypertrial/costguard/tree/main/.github/actions/costguard):

```yaml
- uses: actions/checkout@v4
  with:
    fetch-depth: 0
- uses: ./.github/actions/costguard
  with:
    base: origin/main
    warehouse: snowflake
    fail-on: high
    format: github
```

Inputs: `base`, `warehouse`, `fail-on`, `format` (`github` \| `markdown` \| `json` \| `text`), optional `manifest`, `working-directory`, and dbt compile settings (`compile-dbt`, `dbt-target`, `dbt-project-dir`, `dbt-profiles-dir`, `dbt-adapter-package`).

When `compile-dbt` is enabled (default), the action runs `dbt deps` and `dbt compile`, then passes `--manifest target/manifest.json` when present. Compile uses a dummy local profile (no warehouse connection). Override `dbt-adapter-package` for non-Trino projects (for example `dbt-postgres` for jaffle-shop).

## CI output formats

```bash
costguard pr --base origin/main --warehouse snowflake --fail-on high --format github
costguard pr --base origin/main --warehouse snowflake --fail-on high --format markdown
costguard pr --base origin/main --warehouse snowflake --fail-on high --format json
```

- `github` — annotation commands for GitHub Checks
- `markdown` — PR-summary-oriented report
- `json` — structured `diagnostics` and optional `pr_summary`

See [Output formats](reference/output.md) for the JSON schema.

## Manifest and compiled SQL

For Jinja-heavy dbt models, run `dbt compile` first (or enable compile in the Action). Costguard loads `compiled_code` from the manifest for parse metrics while rule diagnostics stay anchored to raw source lines.

If `--manifest` is omitted, Costguard auto-loads `target/manifest.json` when that file exists in the scan root.

## Related

- [PR check primary workflow](../design/pr-check-primary-workflow.md)
- [Suppressions](reference/suppressions.md)
