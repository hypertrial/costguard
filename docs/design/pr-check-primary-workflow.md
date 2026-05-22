# PR Check Primary Workflow

Automated PR review is Costguard's main use case.

Primary positioning:

> Costguard is a GitHub PR check that catches expensive dbt and warehouse SQL before it merges.

The local Rust CLI is the engine that powers the workflow. It is important for developer debugging,
pre-commit usage, and CI portability, but product decisions should optimize first for PR checks.
The current MVP ships the CLI engine and a composite GitHub Action at
[`.github/actions/costguard`](../../.github/actions/costguard). The PR workflow lives in
[`.github/workflows/costguard-pr.yml`](../../.github/workflows/costguard-pr.yml).

The GitHub Action runs `dbt compile` by default when `dbt_project.yml` is present, then passes
`target/manifest.json` to costguard for compiled SQL parse metrics. Set `compile-dbt: false` to skip.

Workflow:

```text
PR opened
-> changed SQL/dbt files scanned
-> cost/performance risks annotated
-> fail on high-risk findings
-> merge safer analytics code
```

Use-case priority:

| Priority | Use case | Role |
| ---: | --- | --- |
| 1 | GitHub PR check | Main product workflow |
| 2 | Local CLI scan | Developer debugging |
| 3 | Pre-commit hook | Optional fast feedback |
| 4 | CI scheduled scan | Repo hygiene |
| 5 | VS Code/LSP | Later |
| 6 | Query history enrichment | Later advanced mode |

The MVP should optimize this command:

```bash
costguard pr --base origin/main --warehouse snowflake --fail-on high
```

> **Note:** CLI default for `--base` is `main`. CI examples use `origin/main` after checkout with `fetch-depth: 0`.

CI-oriented output formats:

```bash
costguard pr --base origin/main --warehouse snowflake --fail-on high --format github
costguard pr --base origin/main --warehouse snowflake --fail-on high --format markdown
costguard pr --base origin/main --warehouse snowflake --fail-on high --format json
```

`github` emits annotation commands. `markdown` emits a PR-summary-oriented report.
`json` preserves the `diagnostics` array and includes `pr_summary` when PR mode is used.
PR mode should scan changed files first, then use manifest/YAML/SQL graph context only for
blast-radius summaries and rule context. Invalid git bases and non-git directories must fail
the check instead of silently scanning zero files.

Expected PR output should include:

- pass/fail status
- GitHub annotations
- PR summary
- changed-file diagnostics
- optional downstream blast radius
- recommended fixes
- suppression guidance

Example summary:

```text
Costguard failed this PR

2 high-risk cost findings:

1. models/marts/fct_sessions.sql
   Incremental model has no date predicate.
   Risk: likely full table scan on every run.

2. models/staging/stg_events.sql
   Repeated JSON extraction from payload.
   Risk: expensive semi-structured parsing repeated downstream.

Fix these issues or suppress with:
-- costguard: disable-next-line=SQLCOST005
```

MVP pass/fail should be based on risk severity:

```bash
--fail-on high
```

Do not make dollar thresholds the MVP. True cost estimation needs warehouse metadata,
table sizes, query history, pricing models, partitioning/clustering stats, cache behavior,
and execution plans. Add dollar estimates later as enrichment, not as the first gating model.

Suppression directives: prefer SQL comment prefix `-- costguard: ...` (bare `costguard:` also works). See [Suppressions](../book/reference/suppressions.md).
