# Parse metrics

Parse metrics measure SQL/Jinja robustness. They do **not** gate rule execution — shape rules prefer parsed AST features when raw SQL parses, with regex fallback for expression rules and parse failures.

## Model-scoped metrics

Primary fields in JSON output and benchmark baselines:

| Metric | Description |
| --- | --- |
| `sql_parse_total` | Production dbt models scanned (`models/**/*.sql`) |
| `sql_parse_failures` | Models where headline parse failed |
| `sql_parse_compiled_total` | Models with a compiled SQL parse attempt (manifest present) |
| `sql_parse_compiled_failures` | Models where **compiled-only** parse failed |

## Other SQL files

Macros, tests, and non-model SQL use separate counters:

| Metric | Description |
| --- | --- |
| `sql_parse_other_total` | Non-model SQL files |
| `sql_parse_other_failures` | Non-model parse failures |

## Metadata metrics

When dbt metadata is loaded without a manifest or YAML/dbt_project parsing fails:

| Metric | Description |
| --- | --- |
| `metadata_warnings` | Total metadata warning events collected during scan |
| `yaml_parse_failures` | Schema YAML files that failed to parse |
| `dbt_project_parse_failures` | `dbt_project.yml` parse or ambiguous `models:` block failures |
| `metadata_only_scan` | `true` when no manifest was loaded but dbt files were present |

Related diagnostics: `SQLCOST016` (no manifest), `SQLCOST017` (YAML parse failure), `SQLCOST018` (`dbt_project.yml` issue). These are Info/Low severity and do not fail default `--fail-on high` runs.

## Headline vs compiled parse

When a manifest with `compiled_code` is loaded:

1. Costguard normalizes compiled SQL (comment stripping, Trino rewrites, GenericDialect fallback).
2. **Headline** `sql_parse_failures` uses compiled parse when available, with **stripped-raw fallback** when compiled parse fails (`parsed_compiled || parsed_raw`).
3. **`sql_parse_compiled_failures`** tracks compiled parse quality independently — no raw fallback.

Spellbook external baseline requires **`sql_parse_compiled_failures = 0`**.

## Audit gate

Audit compiled parse failures against a merged manifest:

```bash
python3 scripts/audit_compiled_parse_failures.py path/to/manifest.json --bucket
```

See [Scripts](scripts.md) for flags on the underlying `audit-compiled-parse` binary.

## Trino / Spellbook

For Dune Spellbook and other Trino dbt projects:

- Use `--warehouse trino`
- Compile all subprojects and merge manifests (benchmark script automates this)
- Gate on zero compiled failures before refreshing baselines

## Related

- [Benchmark calibration](../design/benchmark-calibration.md)
- [Spellbook stress test](../design/spellbook-stress-test.md)
- [Platforms](platforms.md)
