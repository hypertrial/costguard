#!/usr/bin/env bash
set -eo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

SPELLBOOK_SMOKE=0
for arg in "$@"; do
  case "$arg" in
    --spellbook-smoke) SPELLBOOK_SMOKE=1 ;;
    *) echo "unknown argument: $arg" >&2; exit 2 ;;
  esac
done

run() {
  echo "+ $*"
  "$@"
}

run python3 scripts/validate_workspace_deps.py
run cargo fmt --check
run cargo clippy --all-targets --all-features -- -D warnings
run cargo build -p costguard-cli
run cargo build --release -p costguard-cli
run python3 scripts/verify_release_assets.py
run python3 -m unittest discover -s scripts/tests -p 'test_*.py'
run python3 scripts/validate_fp_registry.py
COSTGUARD_BUILD_PROFILE=release run python3 scripts/benchmark_external_repo.py --all-vendored
run python3 scripts/generate_rule_docs.py --check
if command -v mdbook >/dev/null 2>&1; then
  run mdbook build
else
  echo "WARN: mdbook not installed; skipping mdbook build"
fi
if command -v cargo-deny >/dev/null 2>&1; then
  run cargo deny check
else
  echo "WARN: cargo-deny not installed; skipping cargo deny check"
fi
run cargo test
if [ "$SPELLBOOK_SMOKE" -eq 1 ]; then
  COSTGUARD_BUILD_PROFILE=release run python3 scripts/benchmark_external_repo.py --repo spellbook --smoke
fi
