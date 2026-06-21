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
# min_confidence_filter = true  # optional; hide findings below min_confidence from output

[dbt]
manifest_path = "target/manifest.json"
max_manifest_bytes = 536870912

[owners]
codeowners = true
default = "@data-platform"

[gate]
fail_on = "high"
min_confidence = "high"
require_owner = true

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
| `min_confidence_filter` | boolean | When `true`, omit findings below `min_confidence` from output (default `false`) |
| `baseline` | string | Finding baseline JSON path for grandfathering |

## `[dbt]`

| Key | Type | Description |
| --- | --- | --- |
| `manifest_path` | string | Head manifest for compiled SQL metrics |
| `base_manifest_path` | string | Optional production/state manifest for PR base-vs-head comparison (falls back to the git base ref when unset) |
| `max_manifest_bytes` | integer | Maximum bytes loaded from any head or base manifest (default `536870912`, 512 MiB; omitted or `0` uses the default) |

Manifest limits are enforced with a metadata preflight and a bounded read. Oversized head or base manifests fail the run without emitting a partial PR delta.

## `[owners]`

Owner resolution is additive metadata for diagnostics and PR receipts. Precedence is: dbt model `meta.owner`/`meta.owners`, Costguard tag and path maps, CODEOWNERS, dbt group, then `default`.

| Key | Type | Description |
| --- | --- | --- |
| `default` | string or string array | Fallback owner(s) |
| `codeowners` | bool | Read the first CODEOWNERS file from `.github/`, repository root, or `docs/`; last matching rule wins |
| `[owners.paths]` | glob to owner(s) map | Repository-relative model path routing |
| `[owners.tags]` | tag to owner(s) map | dbt tag routing |

## `[gate]` and `[[gate.scopes]]`

PR-only policy-as-code gates run after signed policy, waivers, baselines, lineage, and cost attribution. Existing `[output].fail_on` remains unchanged. A `mode = "warn"` gate records a warning but does not change the exit code.

| Key | Type | Description |
| --- | --- | --- |
| `name` | string | Receipt label; global default is `default` |
| `mode` | string | `block` (default) or `warn` |
| `fail_on` | severity | Fail/warn when an actionable finding meets the severity threshold |
| `min_confidence` | confidence | Optional floor used with `fail_on` |
| `fail_on_monthly_delta` | positive float | Addressable finding savings threshold in USD/month |
| `fail_on_monthly_delta_gb` | positive float | Addressable finding savings threshold in GB-months |
| `fail_on_blast_radius` | positive integer | Unique downstream-model threshold |
| `require_owner` | bool | Require matched changed model/finding paths to resolve an owner |
| `block_only_new` | bool | Reserved: when enabled in a future release, gates will block only introduced/regressed PR findings (default `false`; advisory preview only today) |

Each `[[gate.scopes]]` requires a unique `name` and at least one matcher: `paths`, `tags`, or `owners`. Values within a matcher are ORed; different matcher categories are ANDed. Baselined and excepted findings do not participate.

## `[[waivers]]`

Local waivers are lightweight, expiring enforcement exceptions. Required fields are `id`, exactly one of `finding_id` or `rule_id`, `path`, `owner`, `reason`, `ticket_url`, `approver`, `created_at`, and `expires_at`. Paths are repository-relative globs and timestamps are RFC3339.

```toml
[[waivers]]
id = "CG-123"
rule_id = "SQLCOST012"
path = "models/finance/**"
owner = "@finance"
reason = "approved migration window"
ticket_url = "https://tracker.example/CG-123"
approver = "@data-lead"
created_at = "2026-06-01T00:00:00Z"
expires_at = "2026-07-01T00:00:00Z"
```

Active matches are emitted with `enforcement = "excepted"`. An expired waiver creates `analysis.violations[].code = "expired_waiver"`. Signed policy rejects local waivers unless `permissions.allow_local_waivers = true`.

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
| `require_manifest_integrity` | bool | Reserved: when enabled in a future release, fail when changed model checksums do not match the head manifest (default `false`; advisory preview only today) |
| `max_parse_failure_rate` | float | Maximum allowed SQL parse failure rate (`0.0`–`1.0`) |
| `max_compiled_parse_failures` | integer | Maximum allowed compiled-SQL parse failures |
| `max_skipped_files` | integer | Maximum allowed skipped or unreadable files |
| `fail_on_metadata_errors` | bool | Fail when YAML or dbt project metadata cannot be parsed |

When `policy = "strict"`, Costguard enforces manifest presence, manifest freshness (not older than modified model SQL files), zero parse failures, zero skipped files, and metadata errors regardless of the other thresholds above.

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
| `min_mapped_spend_fraction` | float | Fail when mapped-spend coverage (`coverage.mapped_spend_fraction`) is below threshold (0.0–1.0); opt-in |
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
