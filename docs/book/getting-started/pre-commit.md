# Pre-commit hook

Costguard ships an optional [pre-commit](https://pre-commit.com/) hook for fast local feedback before push. This is a **secondary workflow** relative to the GitHub PR check.

## Install (consumer repos)

Add to `.pre-commit-config.yaml`:

```yaml
repos:
  - repo: https://github.com/hypertrial/costguard
    rev: v2.7.0
    hooks:
      - id: costguard-pr
```

Then:

```bash
pip install pre-commit
pre-commit install
pre-commit install --hook-type pre-push
```

Install a checksum-verified release binary on your `PATH` as shown in the [quick start](quick-start.md).

## Environment overrides

| Variable | Default | Description |
| --- | --- | --- |
| `COSTGUARD_BASE` | `HEAD~1` | Git base ref for changed-file detection |
| `COSTGUARD_WAREHOUSE` | `snowflake` | Warehouse platform |
| `COSTGUARD_FAIL_ON` | `high` | Minimum failing severity |
| `COSTGUARD_MIN_CONFIDENCE` | `high` | Confidence floor for fail logic |

## Dogfooding in this repo

This repository includes [`.pre-commit-config.yaml`](../../../.pre-commit-config.yaml) pointing at the local hook script for development.

Run the full local qualification gate with:

```bash
./scripts/ci_local.sh
```

Track workflow priority in the [PR check workflow design doc](../../design/pr-check-primary-workflow.md) (pre-commit is priority 3).
