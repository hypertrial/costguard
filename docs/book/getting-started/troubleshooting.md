# Troubleshooting

Common Costguard failures and how to fix them. See also [Requirements](requirements.md), [CLI reference](../reference/cli.md), and [Parse metrics](../reference/parse-metrics.md).

## Stale or missing manifest

**Symptoms:** `SQLCOST045` (stale manifest), `SQLCOST023` (manifest required), or strict-policy `manifest_stale` / `manifest_required` violations.

**Cause:** Costguard loaded `target/manifest.json`, but model SQL is newer than the manifest, or strict mode requires a manifest that is absent.

**Fix:**

```bash
dbt compile && costguard scan
```

In CI, run `dbt compile` before Costguard so compiled SQL and manifest metadata match the PR. See [Requirements](requirements.md).

## Manifest exceeds the configured limit

**Symptoms:** Exit code `3` with an error naming a manifest path, its observed size, and `[dbt].max_manifest_bytes`.

**Cause:** A head, explicit base, or git-base manifest exceeds the 512 MiB default. Costguard stops rather than emitting a partial or misleading finding delta.

**Fix:** Confirm the file is the intended dbt manifest and remove unexpected/generated bloat. For a legitimate larger project, set an explicit reviewed limit:

```toml
[dbt]
max_manifest_bytes = 805306368 # 768 MiB
```

## SQL parse failures

**Symptoms:** `SQLCOST027` (SQL parse failure), high `sql_parse_failures` in metrics, or strict-policy `parse_failure_rate` violations.

**Cause:** Jinja-heavy models without a successful compile, unsupported dialect syntax, or malformed SQL.

**Fix:**

1. Run `dbt compile` and scan again so Costguard can use compiled SQL.
2. Pass `--warehouse` matching your warehouse (`snowflake`, `bigquery`, `trino`, etc.).
3. Use `costguard explain path/to/model.sql` to debug a single file.
4. Relax `--analysis-policy` from `strict` if you accept partial parse coverage on legacy repos.

Regex-only feature extraction still runs when parsing fails; those findings emit `confidence: low`.

## Too many findings on macro-heavy repos

**Symptoms:** PR gate fails on join or shape rules that look like false positives on Jinja macros.

**Cause:** Regex fallbacks fire when SQL does not parse to an AST.

**Fix:** Pair severity and confidence floors:

```bash
costguard pr --base origin/main --fail-on high --min-confidence high
```

Add `--min-confidence-filter` to omit low-confidence findings from output entirely (they still do not fail the gate when below `--min-confidence`).

In `costguard.toml`:

```toml
[output]
fail_on = "high"
min_confidence = "high"
min_confidence_filter = true
```

## PR passes locally but fails in CI

**Checklist:**

- CI must `fetch-depth: 0` so `costguard pr --base origin/main` can diff against the base branch.
- Run `dbt compile` in CI before Costguard when models use Jinja.
- Pin the same `--warehouse`, `--fail-on`, and `--min-confidence` flags locally and in the Action.
- Unchanged-file parse failures appear in the PR `context` report only; they do not fail the gate. Fix them on the default branch separately.

## Exit codes

| Code | Meaning | Typical fix |
| --- | --- | --- |
| `0` | Pass | — |
| `1` | Findings at/above `--fail-on` (and `--min-confidence` when set), analysis incomplete under strict policy, or cost gate exceeded | Fix findings, recompile manifest, or tune gates |
| `2` | Config error | Fix `costguard.toml`, CLI flags, or baseline/policy schema version |
| `3` | Runtime error | Check paths, git state, and file permissions |

Full reference: [Output formats and exit codes](../reference/output.md).

## Still stuck?

- [Local scan and explain](local-scan.md) — single-file debugging
- [Quick start (PR check)](quick-start.md) — CI wiring
- [Rule catalog](../rules/index.md) — per-rule fix guidance
- [Contributing](../../../CONTRIBUTING.md) — open an issue or PR
