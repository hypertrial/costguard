# Project agent notes

Public Rust + Python Costguard tool. Canonical local gates live in `scripts/ci_local.sh`.

## GitHub Actions cost policy

GitHub Actions is intentionally cost-constrained for the organization's free tier. Do not add workflows or artifact uploads without explicit owner approval. Keep CI and benchmark artifacts at one day, release artifacts at no more than seven days, and set `retention-days` explicitly on every `upload-artifact` step.

## Verification

- Fast: `./scripts/ci_local.sh --fast`
- Completion: `./scripts/ci_local.sh`

Release versions use `python3 scripts/release_check.py --version <version>`. Follow CONTRIBUTING.

## Invariants

Public repository. Keep Pad data out of git.
