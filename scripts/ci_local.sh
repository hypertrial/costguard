#!/usr/bin/env bash
set -eo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

SPELLBOOK_SMOKE=0
PRECISION_GATE=0
for arg in "$@"; do
  case "$arg" in
    --spellbook-smoke) SPELLBOOK_SMOKE=1 ;;
    --precision) PRECISION_GATE=1 ;;
    *) echo "unknown argument: $arg" >&2; exit 2 ;;
  esac
done

run() {
  echo "+ $*"
  "$@"
}

run python3 scripts/validate_workspace_deps.py
if command -v ruff >/dev/null 2>&1; then
  run ruff check scripts .github/actions/costguard/scripts
else
  echo "WARN: ruff not installed; skipping ruff check"
fi
run cargo fmt --check
run cargo clippy --locked --all-targets --all-features -- -D warnings
run cargo build --locked -p costguard-cli
run cargo build --release --locked -p costguard-cli
run python3 scripts/verify_release_assets.py
run python3 -m unittest discover -s scripts/tests -p 'test_*.py'
run python3 scripts/validate_fp_registry.py
run python3 scripts/recall_report.py
run python3 scripts/generate_recall_corpus.py --check
COSTGUARD_BUILD_PROFILE=release run python3 scripts/benchmark_external_repo.py --all-vendored
run python3 scripts/generate_rule_docs.py --check
run python3 scripts/check_docs.py
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
run cargo test --workspace --all-targets --locked
if [ "$SPELLBOOK_SMOKE" -eq 1 ]; then
  COSTGUARD_BUILD_PROFILE=release run python3 scripts/benchmark_external_repo.py --repo spellbook --smoke
fi
if [ "$PRECISION_GATE" -eq 1 ]; then
  SPELLBOOK_CACHE="${HOME}/.cache/costguard/benchmarks/spellbook"
  if [ -f "${SPELLBOOK_CACHE}/target/manifest.json" ]; then
    run python3 scripts/precision_triage.py --repo spellbook --sample-size 200
  else
    echo "WARN: spellbook cache missing; skipping precision gate"
  fi
fi
