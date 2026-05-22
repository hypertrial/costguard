# Local scan and explain

Use local commands for debugging rules, reproducing CI findings, and exploring a full project.

## Scan

```bash
costguard scan models/ --warehouse snowflake
costguard scan models/ --warehouse bigquery --format json
costguard scan models/ --format github
costguard scan models/ --manifest target/manifest.json --warehouse trino
```

| Behavior | Detail |
| --- | --- |
| Empty `paths` | Scans the entire current working directory |
| `--manifest` | Optional; auto-discovers `target/manifest.json` when present |
| `--dialect` | Alias for `--warehouse` |
| `--fail-on` | Exit code `1` when diagnostics meet or exceed this severity |

## Explain

Run rules against a single file:

```bash
costguard explain models/marts/fct_orders.sql --warehouse snowflake
costguard explain models/marts/fct_orders.sql --manifest target/manifest.json --format json
```

`explain` supports the same platform and output flags as `scan`, but does not accept `--fail-on`.

## List rules

```bash
costguard rules
costguard rules --format json
costguard rules --format markdown
```

See the [Rule catalog](../rules/index.md) for prose fix guidance per rule.

## Exit codes

| Code | Meaning |
| --- | --- |
| `0` | Completed without diagnostics at or above the fail threshold |
| `1` | Diagnostics met or exceeded the fail threshold |
| `2` | Configuration error |
| `3` | Runtime error |

## Related

- [CLI reference](reference/cli.md)
- [Configuration](reference/configuration.md)
