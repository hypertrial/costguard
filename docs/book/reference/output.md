# Output formats and exit codes

## Text (default)

Human-readable diagnostics with severity, rule id, file location, message, and confidence. When `[cost]` is enabled, each finding includes an estimated savings line and a **Cost summary** section with deduplicated project totals.

## JSON

Structured scan result:

```json
{
  "schema_version": 2,
  "analysis": { "...": "..." },
  "metrics": { "...": "..." },
  "cost": { "...": "..." },
  "diagnostics": [ "..." ],
  "files": [ "..." ],
  "pr_summary": { "...": "..." }
}
```

| Field | Present when | Description |
| --- | --- | --- |
| `schema_version` | Always | JSON schema version (`2` adds optional `cost` block) |
| `analysis` | Always | Completeness report: `policy`, `passed`, and optional `violations` with `code`, `message`, `observed`, and `allowed` |
| `metrics` | Always | Scan counters including parse metrics (see [Parse metrics](parse-metrics.md)) |
| `cost` | `[cost]` enabled | Project cost summary: deduplicated model totals, savings sum, top models, grade mix (see [Cost estimates](cost-estimates.md)) |
| `diagnostics` | Always | Array of findings with `rule_id`, `severity`, `message`, `path`, `line`, `confidence`; optional `cost_estimate` when `[cost]` is enabled (`p50_usd_per_month` is **savings**, not model total); compiled-only unmapped findings include `source_provenance`, `compiled_line`, and `compiled_column` |
| `files` | Always | Per-model parse metadata (`parse_input`, `parsed_raw`, `parsed_compiled`, `feature_extraction_used_ast`) |
| `pr_summary` | PR mode | Changed files, optional downstream blast radius |

## GitHub (`github`)

Emits workflow commands for annotations:

```text
::error file=path,line=N,title=RULE_ID::message
```

Compiled-only unmapped diagnostics annotate the raw model path at line 1 and include the compiled SQL location in the annotation message.

## Markdown (`markdown`)

PR-summary-oriented report with grouped findings and suppression guidance footer.

## SARIF (`sarif`)

[SARIF 2.1.0](https://docs.oasis-open.org/sarif/sarif/v2.1.0/sarif-v2.1.0.html) for GitHub Code Scanning, GitLab SAST (`reports: sast`), and Jenkins SARIF plugins.

Includes `tool.driver.rules` metadata and `results` with physical locations. Severity maps to SARIF levels: `error` (High/Critical), `warning` (Medium), `note` (Low/Info).

## Finding baselines

Use `--write-baseline` / `--baseline` (or `[output].baseline` in config) to grandfather known findings in legacy repos. Metrics include:

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
| `0` | Analysis completeness checks passed; no diagnostics at or above `--fail-on` / `fail_on` (and `--min-confidence` / `min_confidence` when set); cost delta gate not exceeded |
| `1` | Analysis completeness checks failed, one or more diagnostics at or above severity threshold with confidence at or above the optional floor, or deduplicated savings cost gate exceeded |
| `2` | Configuration error (invalid config, missing manifest path) |
| `3` | Runtime error |

## Related

- [CLI reference](cli.md)
- [Parse metrics](parse-metrics.md)
- [Suppressions](suppressions.md)
