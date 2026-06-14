# Output formats and exit codes

## Text (default)

Human-readable diagnostics with severity, rule id, file location, message, and confidence. When `[cost]` is enabled, each finding includes an estimated savings line and a **Cost summary** section with deduplicated project totals.

In PR mode, a **Context** footer summarizes unchanged project files: SQL file counts, parse failures, skipped files, and up to five nonblocking context issues.

## JSON

Structured scan result:

```json
{
  "schema_version": 3,
  "identity_scheme": "semantic-v1",
  "analysis": { "...": "..." },
  "metrics": { "...": "..." },
  "cost": { "...": "..." },
  "diagnostics": [ "..." ],
  "files": [ "..." ],
  "pr_summary": { "...": "..." },
  "context": { "...": "..." }
}
```

| Field | Present when | Description |
| --- | --- | --- |
| `schema_version` | Always | JSON schema version (`3` is the Costguard 2.x contract) |
| `identity_scheme` | When semantic identity is active | `"semantic-v1"` — finding IDs are stable across formatting and line movement |
| `analysis` | Always | Completeness report: `policy`, `passed`, and optional `violations` with `code`, `message`, `observed`, and `allowed` |
| `metrics` | Always | Scan counters including parse metrics (see [Parse metrics](parse-metrics.md)) |
| `cost` | `[cost]` enabled | Project cost summary: deduplicated model totals, savings sum, top models, grade mix (see [Cost estimates](cost-estimates.md)) |
| `diagnostics` | Always | Gated findings on changed files in PR mode; full scan findings otherwise. Each entry includes `rule_id`, `severity`, `message`, `path`, `line`, `confidence`, and governance fields (`finding_id`, `evidence_key`); optional `cost_estimate` when `[cost]` is enabled (`p50_usd_per_month` is **savings**, not model total); compiled-only unmapped findings include `source_provenance`, `compiled_line`, and `compiled_column` |
| `files` | Always | Per-model parse metadata (`parse_input`, `parsed_raw`, `parsed_compiled`, `feature_extraction_used_ast`) |
| `pr_summary` | PR mode | Changed files, optional downstream blast radius |
| `context` | PR mode | Nonblocking full-project report (see below) |

### PR `context` report

In PR mode, changed files drive pass/fail. The optional `context` object reports project-wide state without failing the check:

| Field | Description |
| --- | --- |
| `counts` | Scan counters for unchanged project files |
| `sql_parse_total` / `sql_parse_failures` | Parse coverage on context files |
| `sql_parse_other_total` / `sql_parse_other_failures` | Non-target SQL parse metrics |
| `skipped_count` | Files skipped during context discovery |
| `issues` | Compact list of `{ rule_id, path, message }` for unchanged-file problems (for example parse failures on files not in the PR diff) |

Context issues are informational in PR mode. Fix them on the default branch; do not suppress them to pass a PR gate.

## GitHub (`github`)

Emits workflow commands for annotations:

```text
::error file=path,line=N,title=RULE_ID::message
```

Compiled-only unmapped diagnostics annotate the raw model path at line 1 and include the compiled SQL location in the annotation message.

## Markdown (`markdown`)

PR-summary-oriented report with grouped findings, context footer, and suppression guidance.

## SARIF (`sarif`)

[SARIF 2.1.0](https://docs.oasis-open.org/sarif/sarif/v2.1.0/sarif-v2.1.0.html) for GitHub Code Scanning, GitLab SAST (`reports: sast`), and Jenkins SARIF plugins.

Includes `tool.driver.rules`, physical locations, and additive governance properties. Each result records the finding ID, evidence key, confidence, enforcement outcome, policy provenance, applied exception, and optional cost estimate. Run properties retain scan, policy, analysis, metrics, `context`, and `identity_scheme` metadata. Severity maps to SARIF levels: `error` (High/Critical), `warning` (Medium), `note` (Low/Info). The JSON output schema remains version 3.

## Finding baselines

Use `--write-baseline` / `--baseline` (or `[output].baseline` in config) to grandfather known findings in legacy repos. Baselines must be **schema v3** with `identity_scheme: "semantic-v1"`. Metrics include:

| Field | Description |
| --- | --- |
| `baselined_findings` | Findings suppressed by the baseline file |
| `new_findings` | Findings reported after baseline filtering |

Exit code `1` applies when analysis completeness checks fail (`analysis.passed = false`), to **new** findings at or above `--fail-on`, or when deduplicated **savings** p50 on new findings exceeds `--fail-on-cost-delta` (USD) or `fail_on_monthly_delta_gb` (GB-months) when set.

PR markdown output includes a reminder:

```text
Suppress only intentional exceptions with `-- costguard: disable-next-line=RULE`.
```

## Exit codes

| Code | Meaning |
| --- | --- |
| `0` | Analysis completeness checks passed; no diagnostics at or above `--fail-on` / `fail_on` (and `--min-confidence` / `min_confidence` when set); cost delta gate not exceeded; `explain` completed with `analysis.passed = true` |
| `1` | Analysis completeness checks failed, one or more diagnostics at or above severity threshold with confidence at or above the optional floor, or deduplicated savings cost gate exceeded; `explain` with `analysis.passed = false` |
| `2` | Configuration error (invalid config, missing manifest path, unsupported baseline v2 or policy v1, migration failure) |
| `3` | Runtime error |

## Related

- [CLI reference](cli.md)
- [Compatibility policy](compatibility.md)
- [Parse metrics](parse-metrics.md)
- [Suppressions](suppressions.md)
