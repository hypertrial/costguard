# Dependencies

Costguard keeps a small, explicit set of third-party crates. New dependencies require review.

## Allowed workspace dependencies

| Crate | Role |
| --- | --- |
| `sqlparser`, `regex` | SQL parsing and pattern fallbacks |
| `serde`, `serde_json` | Structured output and manifest I/O |
| `serde_yaml` | dbt YAML (`schema.yml`, `dbt_project.yml`); planned replacement |
| `ignore`, `rayon` | Gitignore-aware scan and parallel analysis |
| `anyhow` | CLI and integration error handling |
| `clap` | CLI parsing (trimmed features; no ANSI/color stack) |
| `toml` | `costguard.toml` config |
| `tempfile` | Dev/test fixtures only (not in release binary) |

Internal crates (`costguard-core`, `costguard-sql`, etc.) should prefer workspace deps and avoid adding new external crates without updating the allowlist.

## Adding a dependency

1. Add the crate under `[workspace.dependencies]` in the root `Cargo.toml`.
2. Update `ALLOWED` in [`scripts/validate_workspace_deps.py`](../../../scripts/validate_workspace_deps.py).
3. Explain why in the PR (what cannot be done in-house, maintenance story, transitive impact).
4. Run `cargo deny check` and ensure advisories/licenses pass.

## Policy checks

```bash
cargo deny check
python3 scripts/validate_workspace_deps.py
```

CI runs both on every pull request.

## Audit binary feature

`costguard-sql` gates `serde_json` behind the `audit-bin` feature for the `audit-compiled-parse` tool:

```bash
cargo build -p costguard-sql --bin audit-compiled-parse --features audit-bin
```
