# Pre-commit (planned)

Costguard is designed to support optional pre-commit hooks for fast local feedback before push. This is a **secondary workflow** relative to the GitHub PR check.

## Current status

A packaged pre-commit hook is **not shipped yet**. There is no `.pre-commit-config.yaml` in this repository.

## Recommended approach today

1. Run `costguard pr --base origin/main` locally before opening a PR, or
2. Rely on the [GitHub Action](quick-start.md) as the primary gate.

## Future hook sketch

When implemented, a typical hook would run changed SQL/dbt files only:

```bash
costguard pr --base HEAD~1 --warehouse snowflake --fail-on high
```

Track progress in the [PR check workflow design doc](../design/pr-check-primary-workflow.md) use-case priority table (pre-commit is priority 3).
