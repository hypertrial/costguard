# PR Check Primary Workflow

Automated PR review is Costguard's main use case.

Primary positioning:

> Costguard is a GitHub PR check that catches expensive dbt and warehouse SQL before it merges.

The local Rust CLI is the engine that powers the workflow. It is important for developer debugging,
pre-commit usage, and CI portability, but product decisions should optimize first for PR checks.
The current MVP ships the CLI engine first; a GitHub Action should be the primary packaged workflow.

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
