# Output formats and exit codes

## Text (default)

Human-readable diagnostics with severity, rule id, file location, message, and confidence. When `[cost]` is enabled, each finding includes an estimated savings line and a **Cost summary** section with current/post-fix/potential savings, addressable finding savings (deduplicated), grade mix, top models, and an advisory disclaimer footer.

In PR mode, a **Context** footer summarizes unchanged project files: SQL file counts, parse failures, skipped files, and up to five nonblocking context issues.

## JSON

Structured scan result:

```json
{
  "schema_version": 4,
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
| `schema_version` | Always | JSON schema version (`4` adds structured cost figures; `3` remains valid for consumers that ignore new `cost` fields) |
| `identity_scheme` | When semantic identity is active | `"semantic-v1"` — finding IDs are stable across formatting and line movement |
| `analysis` | Always | Completeness report: `policy`, `passed`, and optional `violations` with `code`, `message`, `observed`, and `allowed` |
| `metrics` | Always | Scan counters including parse metrics (see [Parse metrics](parse-metrics.md)) |
| `cost` | `[cost]` enabled | Project cost summary: mapped USD and full-project GB-month current/post-fix/potential savings, addressable finding savings, coverage, top models, grade mix, and disclaimer (see [Cost estimates](cost-estimates.md)) |
| `diagnostics` | Always | Gated findings on changed files in PR mode; full scan findings otherwise. Entries include governance fields (`finding_id`, `evidence_key`, optional `owners` and `exception`), advisory `rule_precision_tier`, and optional cost data including `prior_basis`, downstream count, and savings |
| `files` | Always | Per-model parse metadata (`parse_input`, `parsed_raw`, `parsed_compiled`, `feature_extraction_used_ast`) |
| `pr_summary` | PR mode, or receipt comparison | Receipt version `2` includes base-vs-head `finding_delta`, macro/source `indirectly_affected`, `manifest_integrity`, optional `rocky_artifact_integrity`, compatibility field `enforcement_preview`, framework-qualified changed model details, gate results, downstream models/exposures, legacy `recommended_dbt_command`, structured `recommended_commands`, and optional trend deltas |
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

### PR finding-delta semantics

`pr_summary.finding_delta` compares only changed targets after rule configuration, inline suppression, semantic identity, governance, owners, waivers, baseline filtering, confidence filtering, and cost annotation. Additions exist on the head only, deletions on the base only, renames use the old base path and new head path, and modifications exist on both sides. Full base-project SQL/dbt files remain analysis context but cannot be reported as resolved unless their base-side path changed.

Both branches use the current invocation's HEAD `costguard.toml`, signed policy, waivers, baseline, confidence, and cost configuration. Historical configuration is not loaded. Baselined findings, signed-policy exceptions, local waivers, and infrastructure/metadata findings do not enter the delta. Head analysis remains the only source of policy/waiver completeness violations and output-metadata diagnostics.

Rocky findings enter the delta only when both sealed artifacts verify against the head and resolved base commits. If the base artifact is missing or invalid in standard mode, head Rocky findings remain visible, `rocky_artifact_integrity.comparison_complete` is `false`, and `unclassified_head_findings` records how many were excluded from delta classification and `block_only_new` enforcement.

Severity increases are `regressed`; deterministic cost increases above floating-point epsilon are also `regressed`; all remaining matched IDs are `unchanged`. Behavioral head diagnostics without semantic identity are excluded from the ID delta but remain blocking in regression-only enforcement so identity failure cannot fail open.

When `block_only_new` is active, introduced/regressed findings participate in severity, confidence, and addressable-savings gates. Unchanged findings remain in JSON and receipts, render with an explicit nonblocking delta label, and become GitHub notices. Resolved findings never block. Required-owner and blast-radius gates continue to evaluate all changed models. JSON remains schema v4 and receipt version 2; `pr_summary.enforcement_preview` is retained for compatibility even though `block_only_new` is active rather than advisory.

## GitHub (`github`)

Emits workflow commands for annotations:

```text
::error file=path,line=N,title=RULE_ID::message
```

Compiled-unmapped diagnostics are emitted as repository-level annotations. Their message retains the source model path and compiled SQL line/column; Costguard does not fabricate a source line.

In regression-only mode, unchanged diagnostics are emitted as `notice`; introduced and regressed diagnostics retain the level selected by severity and governance. The PR summary notice includes introduced, regressed, resolved, and unchanged counts plus priced net monthly impact when available.

## Markdown (`markdown`)

PR-summary-oriented report with grouped findings, context footer, suppression guidance, and (when `[cost]` is enabled in PR mode) a **PR Cost Impact** section before diagnostics: net/introduced/avoided cost delta, efficiency/volume split, blast radius, coverage, addressable savings, grade mix, and top models. Per-finding lines include severity, confidence, precision tier, delta classification, enforcement status, and savings when priced. Pass/fail headings count only findings that can actually block under regression-only mode.

`--summary-file` writes this report without changing stdout. `--receipt-file` writes the same run as JSON v4. `--compare-receipt` reads a prior JSON v4 receipt and adds diagnostic, high-finding, USD, and GB-month trend deltas under `pr_summary.trend`; USD trend is present only when both receipts have complete USD project coverage.

## SARIF (`sarif`)

[SARIF 2.1.0](https://docs.oasis-open.org/sarif/sarif/v2.1.0/sarif-v2.1.0.html) for GitHub Code Scanning, GitLab SAST (`reports: sast`), and Jenkins SARIF plugins.

Includes `tool.driver.rules`, physical locations when source mapping is available, and additive governance properties. Compiled-unmapped results are repository-level and carry `sourcePath`, `compiledLine`, and `compiledColumn` properties instead of a fake physical location. Each result records the finding ID, evidence key, confidence, enforcement outcome, policy provenance, applied exception, and optional cost estimate. Run properties retain scan, policy, analysis, metrics, `context`, and `identity_scheme` metadata. Severity maps to SARIF levels: `error` (High/Critical), `warning` (Medium), `note` (Low/Info). The JSON output schema remains version 4.

## Finding baselines

Use `--write-baseline` / `--baseline` (or `[output].baseline` in config) to grandfather known findings in legacy repos. Baselines must be **schema v3** with `identity_scheme: "semantic-v1"`. Metrics include:

| Field | Description |
| --- | --- |
| `baselined_findings` | Findings suppressed by the baseline file |
| `new_findings` | Findings reported after baseline filtering |

Exit code `1` applies when analysis completeness checks fail (`analysis.passed = false`), a blocking gate fails, findings at or above `--fail-on` are enforceable, addressable finding savings reaches `--fail-on-cost-delta` / `fail_on_monthly_delta_gb`, priced PR net cost reaches `fail_on_pr_cost_increase`, or mapped-spend coverage is below `--min-cost-coverage` / `[cost].min_mapped_spend_fraction`. With regression-only enforcement, finding gates count only introduced/regressed diagnostics; model change controls still cover all changed models.

PR markdown output includes a reminder:

```text
Suppress only intentional exceptions with `-- costguard: disable-next-line=RULE`.
```

## Exit codes

| Code | Meaning |
| --- | --- |
| `0` | Analysis completeness checks passed; no diagnostics at or above `--fail-on` / `fail_on` (and `--min-confidence` / `min_confidence` when set); cost delta and optional cost-coverage gates not exceeded; `explain` completed with `analysis.passed = true` |
| `1` | Analysis completeness failed, a blocking scoped gate failed, diagnostics met severity/confidence thresholds, or an enabled cost threshold was exceeded; `explain` with `analysis.passed = false` |
| `2` | Configuration error (invalid config, missing manifest path, unsupported baseline or policy schema) |
| `3` | Runtime error |

For `doctor`, code `0` means no readiness blockers, `1` means one or more blockers, `2` means invalid configuration, and `3` means the readiness scan could not run. `init` remains code `0` after successfully creating or preserving files even when its automatic doctor report contains blockers or warnings.

## Related

- [CLI reference](cli.md)
- [Compatibility policy](compatibility.md)
- [Parse metrics](parse-metrics.md)
- [Suppressions](suppressions.md)
