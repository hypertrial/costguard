# Product scope

Costguard is a local, dbt-aware cost regression guardrail for git workflows.

This document states what is in scope for the MVP, what is explicitly out of scope, and what comes next. It complements the workflow-focused [PR check primary workflow](pr-check-primary-workflow.md) doc.

## In scope (shipped today)

These capabilities are production-ready or actively maintained as part of the current release line.

| Capability | Notes |
| --- | --- |
| **PR check workflow** | `costguard pr`, composite GitHub Action, `costguard init` scaffolding |
| **Local scan and explain** | `costguard scan`, `costguard explain` for developer debugging |
| **44 SQLCOST rules** | Severity and confidence are the enforcement contract |
| **Multi-warehouse parsing** | Generic SQL, Snowflake, BigQuery, Trino (GA); Databricks, Redshift, Postgres, DuckDB (preview) |
| **Advisory cost estimates** | Local priors and offline inputs only — never a live bill |
| **Governance** | Baseline v3, signed policy v2, semantic finding identity (`semantic-v1`), suppressions |
| **Output formats** | `github`, `markdown`, `json` plus documented exit-code contract |
| **Privacy / local-only** | No warehouse credentials, no network calls during scan |
| **Internal eval and benchmark** | Classification metrics pipeline and IRR judge for calibration (see [Classification metrics](classification-metrics.md), [LLM judge IRR](llm-judge-irr.md)) |

Primary workflow priority (from [PR check primary workflow](pr-check-primary-workflow.md)):

| Priority | Use case | Role |
| ---: | --- | --- |
| 1 | GitHub PR check | Main product workflow |
| 2 | Local CLI scan | Developer debugging |
| 3 | Pre-commit hook | Optional fast feedback |
| 4 | CI scheduled scan | Repo hygiene |

Default PR gate:

```bash
costguard pr --base origin/main --warehouse snowflake --fail-on high --min-confidence high
```

Severity and confidence remain the default failure model. Optional cost deltas (`--fail-on-cost-delta`) require calibrated local inputs.

## LLM judge MVP (in scope, early)

An **opt-in, fully-offline, advisory second pass** over Costguard findings. The judge labels each finding as likely true positive, likely false positive, or unsure to help developers triage noise on macro-heavy repos.

This is a **user-facing** capability, distinct from the internal IRR benchmark judge in [LLM judge IRR](llm-judge-irr.md).

### What it does

1. Run a normal Costguard scan or PR check (`costguard scan` / `costguard pr --format json`).
2. For each diagnostic, apply a local LLM rubric adjudication using the same prompt and verdict mapping as the IRR pipeline.
3. Emit an advisory report showing the original finding plus the judge verdict.

Verdicts:

| Judge output | Meaning |
| --- | --- |
| `tp` | Failure condition clearly met; no documented exemption applies |
| `fp` | Exemption applies or failure condition not met |
| `unsure` | SQL context insufficient to judge (abstention) |

### MVP shape

A thin Python entrypoint (planned: `scripts/costguard_judge.py`) that:

1. Invokes `costguard pr` or `costguard scan --format json`.
2. Reuses [`scripts/llm_judge_lib.py`](../../scripts/llm_judge_lib.py):
   - `finding_id_for_diagnostic` — stable finding identity
   - `pack_sql` / `pack_sql_for_file` — deterministic SQL context packing
   - `build_messages` — rubric + few-shot prompts
   - `LlamaJudge.judge` — local GGUF inference via llama-cpp-python
   - `map_structured_verdict` — deterministic post-processing to `tp` / `fp` / `unsure`
3. Writes an advisory report (JSON or markdown) with judge annotations per finding.

No new model plumbing is required for the MVP; the IRR pipeline already implements the judge core.

### Hard constraints

These constraints preserve Costguard's local-only ethos and keep the judge advisory:

| Constraint | Rationale |
| --- | --- |
| **User-provided GGUF** | Model via `COSTGUARD_JUDGE_GGUF`; Costguard does not download or bundle weights |
| **Fully offline** | No network calls during judge inference |
| **Advisory only** | Judge output never auto-suppresses findings or changes exit codes |
| **Never gates CI** | Judge does not run in CI workflows; severity + confidence remain the only enforcement contract |
| **Opt-in** | Developer explicitly runs the judge wrapper; not enabled by default |

### Expected usage (MVP)

```bash
# One-time local judge env
python3 -m venv .venv-judge
.venv-judge/bin/pip install -r requirements-judge.txt

export COSTGUARD_JUDGE_GGUF=/path/to/model.gguf

# Scan, then judge findings locally (planned entrypoint)
.venv-judge/bin/python scripts/costguard_judge.py scan --warehouse snowflake
.venv-judge/bin/python scripts/costguard_judge.py pr --base origin/main --warehouse snowflake
```

The judge is intended for local triage after a scan — not as a replacement for rule severity and confidence gates.

## Out of scope (MVP)

| Item | Notes |
| --- | --- |
| Live warehouse connections | No credentials, no query execution against production |
| Dollar thresholds as primary PR gate | `--fail-on-cost-delta` is optional and requires calibrated inputs |
| LLM judge in CI | Judge never auto-fails PRs or runs in GitHub Actions |
| Bundled model weights | Users supply their own GGUF |
| VS Code / LSP integration | Later use case (priority 5) |
| Query history enrichment | Later advanced mode (priority 6) |

## Roadmap (post-MVP)

| Item | Notes |
| --- | --- |
| Native `--judge` CLI flag | Integrate judge into `costguard pr` / `costguard scan` in Rust CLI |
| κ floor gate | Optional quality gate in `eval_irr.py` once committed labels exist |
| VS Code / LSP | Inline diagnostics and fix hints |
| Query history enrichment | Optional offline query-history exports for sharper cost priors |

## Related docs

- [PR check primary workflow](pr-check-primary-workflow.md) — workflow priorities and PR output expectations
- [LLM judge IRR](llm-judge-irr.md) — internal benchmark judge (calibration, not user-facing)
- [Classification metrics](classification-metrics.md) — operational precision/recall/MCC for rule evaluation
- [Cost estimates](../book/reference/cost-estimates.md) — advisory savings model
- [Privacy and local-only guarantees](../book/getting-started/privacy.md)
