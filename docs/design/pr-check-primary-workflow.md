# PR Check Primary Workflow

Automated dbt PR cost review is Costguard's main use case.

Primary positioning:

> Costguard is the dbt PR cost regression gate for CI.

The local Rust CLI is the engine that powers the workflow. It is important for developer debugging,
pre-commit usage, and CI portability, but product decisions should optimize first for PR checks.
The current MVP ships the CLI engine and a composite GitHub Action at
[`.github/actions/costguard`](../../.github/actions/costguard). The PR workflow lives in
[`.github/workflows/costguard-pr.yml`](../../.github/workflows/costguard-pr.yml).

Run `dbt compile` in your existing CI job before the Action when you want compiled-SQL and lineage context. Costguard auto-detects `target/manifest.json` when present; raw analysis still works without it. The Action does not install or compile dbt and never connects to the warehouse.

Workflow:

```text
PR opened
-> changed SQL/dbt files scanned
-> dbt cost/performance risks annotated
-> optional downstream blast radius and advisory savings reported
-> owners routed and global/scoped policy gates evaluated
-> markdown summary and JSON receipt written from the same scan
-> fail on high-confidence, high-severity findings
-> merge safer analytics code
```

Measured benchmark evidence is part of the product contract: the current public snapshot reports 97.2% overall sampled precision, 99.8% high-severity sampled precision, and 44/44 behavioral rules passing the TP census.

Use-case priority:

| Priority | Use case | Role |
| ---: | --- | --- |
| 1 | GitHub PR check | Main product workflow |
| 2 | Local CLI scan | Developer debugging |
| 3 | Pre-commit hook | Optional fast feedback |
| 4 | CI scheduled scan | Repo hygiene |
| 5 | VS Code/LSP | Later |
| 6 | Query history enrichment | Later advanced mode |

General SQL analyzers cover broader categories such as security, compliance, migrations, app-code SQL extraction, schema validation, autofix, and editor feedback. Costguard should not chase that surface in the PR workflow; it should stay focused on dbt PR cost regression control.

The MVP should optimize this command:

```bash
costguard pr --base origin/main --warehouse snowflake --fail-on high --min-confidence high
```

> **Note:** CLI default for `--base` is `main`. CI examples use `origin/main` after checkout with `fetch-depth: 0`.

CI-oriented output formats:

```bash
costguard pr --base origin/main --warehouse snowflake --fail-on high --min-confidence high --format github
costguard pr --base origin/main --warehouse snowflake --fail-on high --min-confidence high --format markdown
costguard pr --base origin/main --warehouse snowflake --fail-on high --min-confidence high --format json
```

`github` emits annotation commands. `markdown` emits a PR-summary-oriented report. `json` preserves diagnostics and `pr_summary`. `--summary-file` and `--receipt-file` write both artifacts from the same result while stdout retains `--format`; `--compare-receipt` adds trend deltas.
PR mode should scan changed files first, then use manifest/YAML/SQL graph context only for
blast-radius summaries and rule context. Invalid git bases and non-git directories must fail
the check instead of silently scanning zero files.

Expected PR output should include:

- pass/fail status
- GitHub annotations
- PR summary
- owner routing and gate outcomes
- versioned JSON receipt and optional trend
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

MVP pass/fail should be based on risk severity **and** diagnostic confidence on macro-heavy repos:

```bash
--fail-on high
--min-confidence high
```

Use both flags together in PR checks. `--fail-on high` alone still fails on regex-only shape findings (low confidence) when raw SQL does not parse; `--min-confidence high` keeps AST-confirmed high-risk hits and suppresses that noise.

Do not make dollar thresholds the **primary** MVP gate. Severity and confidence remain the default PR failure model. Optional cost estimates (see [Cost estimates](../book/reference/cost-estimates.md)) use local catalog stats, offline query-history exports, and configurable priors—never live warehouse connections. Use `--fail-on-cost-delta` only when you have calibrated inputs; otherwise rely on `--fail-on high --min-confidence high`.

Suppression directives: prefer SQL comment prefix `-- costguard: ...` (bare `costguard:` also works). See [Suppressions](../book/reference/suppressions.md).
