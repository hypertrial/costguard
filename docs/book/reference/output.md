# Output formats and exit codes

## Text (default)

Human-readable diagnostics with severity, rule id, file location, message, and confidence.

## JSON

Structured scan result:

```json
{
  "metrics": { "...": "..." },
  "diagnostics": [ "..." ],
  "pr_summary": { "...": "..." }
}
```

| Field | Present when | Description |
| --- | --- | --- |
| `metrics` | Always | Scan counters including parse metrics (see [Parse metrics](parse-metrics.md)) |
| `diagnostics` | Always | Array of findings with `rule_id`, `severity`, `message`, `path`, `line`, `confidence` |
| `pr_summary` | PR mode | Changed files, optional downstream blast radius |

## GitHub (`github`)

Emits workflow commands for annotations:

```text
::error file=path,line=N,title=RULE_ID::message
```

## Markdown (`markdown`)

PR-summary-oriented report with grouped findings and suppression guidance footer.

PR markdown output includes a reminder:

```text
Suppress only intentional exceptions with `-- costguard: disable-next-line=RULE`.
```

## Exit codes

| Code | Meaning |
| --- | --- |
| `0` | No diagnostics at or above `--fail-on` / `fail_on` |
| `1` | One or more diagnostics at or above threshold |
| `2` | Configuration error (invalid config, missing manifest path) |
| `3` | Runtime error |

## Related

- [CLI reference](cli.md)
- [Parse metrics](parse-metrics.md)
- [Suppressions](suppressions.md)
