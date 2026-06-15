# Configuration

Costguard runs with zero configuration. A bare `costguard scan` from a dbt project root uses sensible defaults: scan the whole project, auto-detect `target/manifest.json` when present, and fail on `high`-severity findings.

Optional `costguard.toml` in the project root overrides those defaults. CLI flags override file settings.

Configuration is strict in 1.0. Unknown sections, fields, rule settings, and rule IDs are configuration errors and exit with code `2`.

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
| `max_file_bytes` | integer | Maximum file size to load during discovery (default 5242880; `0` uses the built-in default) |

## `[output]`

| Key | Type | Description |
| --- | --- | --- |
| `format` | string | `text`, `json`, `github`, `markdown`, `md`, `sarif` |
| `fail_on` | string | Minimum failing severity |
| `min_confidence` | string | Optional confidence floor for fail logic: `low`, `medium`/`med`, `high` |
| `baseline` | string | Finding baseline JSON path for grandfathering |

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

## `[analysis]`

Optional analysis completeness policy. CLI and GitHub Action default is `standard`.

| Key | Type | Description |
| --- | --- | --- |
| `policy` | string | `standard` (best effort) or `strict` (fail closed) |
| `require_manifest` | bool | Require a dbt manifest for dbt projects |
| `max_parse_failure_rate` | float | Maximum allowed SQL parse failure rate (`0.0`–`1.0`) |
| `max_compiled_parse_failures` | integer | Maximum allowed compiled-SQL parse failures |
| `max_skipped_files` | integer | Maximum allowed skipped or unreadable files |
| `fail_on_metadata_errors` | bool | Fail when YAML or dbt project metadata cannot be parsed |

When `policy = "strict"`, Costguard enforces manifest presence, zero parse failures, zero skipped files, and metadata errors regardless of the other thresholds above.

## `[policy]`

Optional signed-policy bundle for git-native governance. See [Signed policy and exceptions](../governance/policy.md).

| Key | Type | Description |
| --- | --- | --- |
| `bundle` | string | Path to a signed policy bundle JSON file |
| `trust_store` | string | Path to the committed public trust store (for example `.costguard/trust.json`) |
| `organization` | string | Organization slug for policy resolution |
| `team` | string | Optional team slug for narrower scope |
| `repository` | string | Repository slug (`owner/repo`) for policy resolution |
| `required` | bool | Fail when the bundle or trust store is missing (default `false`) |

Example:

```toml
[policy]
bundle = "policy/policy.signed.json"
trust_store = ".costguard/trust.json"
organization = "acme"
repository = "acme/warehouse"
required = true
```

CLI flags `--policy`, `--trust-store`, `--policy-organization`, `--policy-team`, and `--policy-repository` override these values.

## `[cost]`

Optional cost-estimation settings. See [Cost estimates](cost-estimates.md) for the full guide.

| Key | Type | Description |
| --- | --- | --- |
| `enabled` | bool | Enable cost annotations (default `true` when section present) |
| `interval` | float | Interval coverage for p10/p90 (default `0.80`) |
| `default_runs_per_month` | float | Default model run frequency (default `30`) |
| `default_table_size` | string | Size prior when bytes unknown: `small`, `medium`, `large`, `xlarge` |
| `fail_on_monthly_delta` | float | Fail when **addressable finding savings** p50 on new findings exceeds threshold (USD) |
| `fail_on_monthly_delta_gb` | float | Fail when addressable finding savings in GB-months on new findings exceeds threshold |
| `incremental_fraction` | float | Fraction of table bytes for incremental models (default `0.05`) |

### `[cost.pricing]`

| Key | Type | Description |
| --- | --- | --- |
| `model` | string | `scan` or `compute`; omit for relative index only |
| `usd_per_tb` | float | Scan-priced warehouses (BigQuery on-demand, etc.) |
| `usd_per_credit` | float | Compute-priced warehouses |
| `tb_per_credit_hour` | float or `{ p10, p90 }` | Bytes-to-credit conversion (compute regime) |

### `[cost.inputs]`

| Key | Type | Description |
| --- | --- | --- |
| `catalog` | string | Path to dbt `catalog.json` (`stats.num_bytes`) |
| `query_history` | string | Offline CSV export (`model_or_table`, `bytes_per_run`) |
| `observations` | string | Normalized `CostObservationBundleV1` JSON (preferred Grade-A input; USD > credits > bytes) |
| `observations_before` | string | Pre-change observation bundle for realized-savings verification |
| `observations_after` | string | Post-change observation bundle for realized-savings verification |

### `[cost.sources."name"]` / `[cost.models."name"]` / `[cost.rules.RULE_ID]`

Override bytes, runs per month, or rule multiplier priors. Source keys match dbt source or ref names.

## Related

- [Signed policy and exceptions](../governance/policy.md)
- [Cost estimates](cost-estimates.md)
- [CLI reference](cli.md)
- [Rule catalog](../rules/index.md)
- [Compatibility](compatibility.md)
