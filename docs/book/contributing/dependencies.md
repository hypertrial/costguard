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
| `tempfile` | Dev/test fixtures and same-directory atomic CLI key persistence |

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

The local CI gate runs both for developer feedback. GitHub Actions is the authoritative publication and hosted qualification environment.

## Python locks

`requirements-eval.txt` and `requirements-judge.txt` contain direct maintainer inputs. Their pip-compatible `.lock` files are authoritative for installation and include universal hashes for Python 3.11 and newer. Regenerate both with maintainer-installed `uv` and verify them offline with:

```bash
python3 scripts/lock_python_deps.py
python3 scripts/lock_python_deps.py --check
```

CI uses standard pip with `--require-hashes`; `uv` is not a CI dependency.

## Audit binary feature

`costguard-sql` gates `serde_json` behind the `audit-bin` feature for the `audit-compiled-parse` tool:

```bash
cargo build -p costguard-sql --bin audit-compiled-parse --features audit-bin
```
