# Configuration

Costguard reads optional `costguard.toml` from the project root. CLI flags override file settings.

## Example

```toml
warehouse = "snowflake"
# dialect = "snowflake"  # optional alias; warehouse wins when both are set

[scan]
paths = ["models"]
ignore = ["target", "dbt_packages"]

[output]
format = "text"
fail_on = "high"
# min_confidence = "high"  # optional; suppress regex-only findings in PR gates

[dbt]
manifest_path = "target/manifest.json"

[rules.SQLCOST002]
threshold = 3

[rules.SQLCOST004]
enabled = true
severity = "high"
```

## Top-level keys

| Key | Type | Description |
| --- | --- | --- |
| `warehouse` | string | Platform for SQL parsing (preferred key) |
| `dialect` | string | Alias for `warehouse`; used only when `warehouse` is unset |

## `[scan]`

| Key | Type | Description |
| --- | --- | --- |
| `paths` | string array | Subdirectories/files to scan (empty = entire root) |
| `ignore` | string array | Paths to skip (relative to root) |

## `[output]`

| Key | Type | Description |
| --- | --- | --- |
| `format` | string | `text`, `json`, `github`, `markdown`, `md` |
| `fail_on` | string | Minimum failing severity |
| `min_confidence` | string | Optional confidence floor for fail logic: `low`, `medium`/`med`, `high` |

## `[dbt]`

| Key | Type | Description |
| --- | --- | --- |
| `manifest_path` | string | Default manifest for compiled SQL metrics |

## `[rules.RULE_ID]`

Rule IDs are normalized to uppercase (for example `SQLCOST002`).

| Key | Type | Description |
| --- | --- | --- |
| `enabled` | bool | Disable a rule when `false` |
| `severity` | string | Override default severity for the rule |
| `threshold` | integer | Minimum count before some repetition rules fire (default varies by rule; often `2`) |

## Related

- [CLI reference](cli.md)
- [Rule catalog](../rules/index.md)
