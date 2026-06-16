# Local scan and explain

The fastest way to run Costguard on a dbt project is a bare `costguard scan` from the project root. No `costguard.toml`, no flags, and no warehouse credentials required.

See [Requirements](requirements.md) for manifest, git, and compile guidance. Use `explain` to debug a single file, or add flags to refine dialect, output format, and fail thresholds. See also [Quick start (PR check)](quick-start.md) for the CI workflow.

## Scan

```bash
costguard scan
```

With no path arguments, Costguard scans the whole project root. Findings at or above `high` severity fail the run. Manifest behavior: [Requirements](requirements.md).

### Refine with flags

```bash
costguard scan --warehouse snowflake
costguard scan models/ --warehouse bigquery --format json
costguard scan models/ --format github
costguard scan --warehouse trino --manifest target/manifest.json
```

| Flag | Notes |
| --- | --- |
| `--warehouse` / `--dialect` | Sharper parsing for your warehouse. Recommended when you know the dialect. |
| `--manifest` | Optional; see [Requirements](requirements.md) |
| `--fail-on` | Exit code `1` when diagnostics meet or exceed this severity (default: `high`) |

Run `dbt compile` before scanning when you want manifest-backed analysis on Jinja-heavy models. See [Requirements](requirements.md).

## Local iteration

For laptop workflows, compile then scan in one step:

```bash
dbt compile && costguard scan
```

If you edit models without recompiling, Costguard emits **SQLCOST045** (stale manifest). Under `--analysis-policy strict`, that fails the run until you recompile.

Optional pre-commit hook (uses your installed dbt and profiles):

```yaml
repos:
  - repo: local
    hooks:
      - id: costguard
        name: costguard compile + scan
        entry: bash -c 'dbt compile && costguard scan'
        language: system
        pass_filenames: false
        files: \.(sql|yml|yaml)$
```

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

- [CLI reference](../reference/cli.md)
- [Configuration](../reference/configuration.md)
