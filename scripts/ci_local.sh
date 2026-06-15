#!/usr/bin/env bash
set -eo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

SPELLBOOK_SMOKE=0
DATA_INFRA_SMOKE=0
PRECISION_GATE=0
for arg in "$@"; do
  case "$arg" in
    --spellbook-smoke) SPELLBOOK_SMOKE=1 ;;
    --data-infra-smoke) DATA_INFRA_SMOKE=1 ;;
    --precision) PRECISION_GATE=1 ;;
    *) echo "unknown argument: $arg" >&2; exit 2 ;;
  esac
done

run() {
  echo "+ $*"
  "$@"
}

require_tool() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "ERROR: required tool not installed: $1" >&2
    exit 2
  fi
}

require_tool ruff
require_tool mdbook
require_tool cargo-deny
run python3 scripts/validate_workspace_deps.py
run ruff check scripts .github/actions/costguard/scripts
run cargo fmt --check
run cargo clippy --locked --all-targets --all-features -- -D warnings
RUSTDOCFLAGS="-D warnings" run cargo doc --workspace --no-deps --locked
run cargo build --release --locked -p costguard-cli
run python3 scripts/verify_release_assets.py
run python3 -m unittest discover -s scripts/tests -p 'test_*.py'
run python3 scripts/validate_fp_registry.py
run python3 scripts/recall_report.py
COSTGUARD_BUILD_PROFILE=release run python3 scripts/benchmark_external_repo.py --all-vendored
run python3 scripts/generate_rule_docs.py --check
run python3 scripts/check_docs.py
run mdbook build
run cargo deny check
run cargo test --workspace --all-targets --locked
if [ "$SPELLBOOK_SMOKE" -eq 1 ]; then
  COSTGUARD_BUILD_PROFILE=release run python3 scripts/benchmark_external_repo.py --repo spellbook --smoke
fi
if [ "$DATA_INFRA_SMOKE" -eq 1 ]; then
  COSTGUARD_BUILD_PROFILE=release run python3 scripts/benchmark_external_repo.py --repo data-infra --smoke
fi
if [ "$PRECISION_GATE" -eq 1 ]; then
  SPELLBOOK_CACHE="${HOME}/.cache/costguard/benchmarks/spellbook"
  if [ -f "${SPELLBOOK_CACHE}/target/manifest.json" ]; then
    run python3 scripts/precision_triage.py --repo spellbook --sample-size 200
  else
    echo "ERROR: spellbook cache missing; precision gate cannot run" >&2
    exit 2
  fi
fi
