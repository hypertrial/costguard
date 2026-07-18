# Product scope

Costguard is the dbt PR cost regression gate for CI. It reviews changed models before merge, uses optional dbt manifest and lineage context for downstream impact, and runs without warehouse credentials.

This document states what is in scope for the MVP, what is explicitly out of scope, and what comes next. It complements the workflow-focused [PR check primary workflow](pr-check-primary-workflow.md) doc.

## In scope (shipped today)

These capabilities are production-ready or actively maintained as part of the current release line.

| Capability | Notes |
| --- | --- |
| **PR cost regression gate** | Base/head finding classification, regression-only enforcement, and project net-cost gates through `costguard pr`, the composite Action, and `costguard init` |
| **Local scan and explain** | `costguard scan`, `costguard explain` for developer debugging |
| **Sealed Rocky analysis** | Hash-verified expanded SQL, mixed dbt/Rocky lineage, and exact-base PR comparison without linking or invoking Rocky |
| **47 SQLCOST rules** | Severity and confidence are the enforcement contract |
| **Multi-warehouse parsing** | Generic SQL, Snowflake, BigQuery, Trino (GA); Databricks, Redshift, Postgres, DuckDB (preview) |
| **Advisory cost estimates** | Local priors and offline inputs only — never a live bill |
| **Offline cost enrichment** | dbt catalog, query-history CSV, and normalized observation bundles; no live warehouse access |
| **Governance** | Baseline v3, signed policy v2, semantic finding identity, owner routing, scoped gates, and expiring waivers |
| **PR receipts** | One-scan GitHub annotations, markdown summary, JSON v4 receipt, and optional trend comparison |
| **Output formats** | `github`, `markdown`, `json`, `sarif` plus documented exit-code contract |
| **Privacy / local-only** | No warehouse credentials, no network calls during scan |
| **Measured benchmark evidence** | Public precision snapshot plus classification metrics pipeline and IRR judge for calibration (see [Benchmark evidence](../book/reference/benchmarks.md), [Classification metrics](classification-metrics.md), [LLM judge IRR](llm-judge-irr.md)) |

Primary workflow priority (from [PR check primary workflow](pr-check-primary-workflow.md)):

| Priority | Use case | Role |
| ---: | --- | --- |
| 1 | GitHub PR check | Main product workflow |
| 2 | Local CLI scan | Developer debugging |
| 3 | Pre-commit hook | Optional fast feedback |
| 4 | CI scheduled scan | Repo hygiene |

Default PR gate:

```bash
costguard pr --base origin/main --warehouse snowflake --fail-on high --min-confidence high --block-only-new
```

Severity and confidence remain the default failure model. `--fail-on-cost-delta` is the addressable finding-savings gate; `--fail-on-pr-cost-increase` is the separate project-wide net PR cost gate. Both dollar gates require calibrated local inputs.

Costguard is intentionally narrower than general SQL analyzers. Broad SQL security/compliance linting, migration analysis, app-code SQL extraction, schema validation, autofix, and editor feedback are adjacent categories; Costguard's product surface is dbt PR cost regression control, downstream blast radius, advisory savings, and credential-free CI.

## Offline LLM judge (advisory and deferred)

The existing local LLM tooling supports internal benchmark inter-rater reliability. A user-facing judge remains deferred and outside the primary PR workflow. If pursued, it must stay opt-in, fully offline, user-model-supplied, and advisory: it may label findings for local triage but must not suppress diagnostics, change exit codes, or run in the default Action. See [LLM judge IRR](llm-judge-irr.md).

## Out of scope (MVP)

| Item | Notes |
| --- | --- |
| Live warehouse connections | No credentials, no query execution against production |
| Dollar thresholds as primary PR gate | `--fail-on-cost-delta` is optional and requires calibrated inputs |
| LLM judge in CI | Judge never auto-fails PRs or runs in GitHub Actions |
| Bundled model weights | Users supply their own GGUF |
| Broad SQL analyzer replacement | Security/compliance linting, migrations, app-code SQL extraction, schema validation, autofix, and editor-first linting are not the primary product |

## Prioritized follow-on work

| Item | Notes |
| --- | --- |
| Cost evidence calibration | Improve coverage and interval validation for offline Grade A/B inputs |
| Change-control fidelity | Deepen dbt state, lineage, ownership, and model-level PR impact evidence |
| Release evidence | Keep real-repo precision, TP census, PR replay, and cost-impact proof reproducible |

## Related docs

- [PR check primary workflow](pr-check-primary-workflow.md) — workflow priorities and PR output expectations
- [LLM judge IRR](llm-judge-irr.md) — internal benchmark judge (calibration, not user-facing)
- [Classification metrics](classification-metrics.md) — operational precision/recall/MCC for rule evaluation
- [Cost estimates](../book/reference/cost-estimates.md) — advisory savings model
- [Privacy and local-only guarantees](../book/getting-started/privacy.md)
