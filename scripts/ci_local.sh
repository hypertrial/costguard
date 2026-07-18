#!/usr/bin/env bash
set -eo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

SPELLBOOK_SMOKE=0
NBA_MONTE_CARLO_SMOKE=0
PRECISION_GATE=0
CENSUS_GATE=0
FAST_GATE=0
for arg in "$@"; do
  case "$arg" in
    --fast) FAST_GATE=1 ;;
    --spellbook-smoke) SPELLBOOK_SMOKE=1 ;;
    --nba-monte-carlo-smoke) NBA_MONTE_CARLO_SMOKE=1 ;;
    --precision) PRECISION_GATE=1 ;;
    --census) CENSUS_GATE=1 ;;
    *) echo "unknown argument: $arg" >&2; exit 2 ;;
  esac
done

TRACKED_DIFF_BEFORE="$(mktemp)"
TRACKED_DIFF_AFTER="$(mktemp)"
git diff --binary --no-ext-diff HEAD -- > "$TRACKED_DIFF_BEFORE"
check_tracked_diff() {
  local status=$?
  trap - EXIT
  git diff --binary --no-ext-diff HEAD -- > "$TRACKED_DIFF_AFTER"
  if ! cmp -s "$TRACKED_DIFF_BEFORE" "$TRACKED_DIFF_AFTER"; then
    echo "ERROR: local CI mutated tracked files; newly introduced diff follows" >&2
    if ! diff -u "$TRACKED_DIFF_BEFORE" "$TRACKED_DIFF_AFTER" >&2; then
      :
    fi
    status=1
  fi
  rm -f "$TRACKED_DIFF_BEFORE" "$TRACKED_DIFF_AFTER"
  exit "$status"
}
trap check_tracked_diff EXIT

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

eval_python() {
  local lock_digest python_version fingerprint marker
  lock_digest="$(python3 -c 'import hashlib, pathlib, sys; print(hashlib.sha256(pathlib.Path(sys.argv[1]).read_bytes()).hexdigest())' "${ROOT}/requirements-eval.lock")"
  python_version="$(python3 -c 'import platform; print(platform.python_version())')"
  fingerprint="python=${python_version};lock=${lock_digest}"
  marker="${ROOT}/.venv-eval/.costguard-lock-fingerprint"
  if [ ! -x "${ROOT}/.venv-eval/bin/python" ] \
    || [ ! -f "${marker}" ] \
    || [ "$(cat "${marker}" 2>/dev/null || true)" != "${fingerprint}" ]; then
    rm -rf "${ROOT}/.venv-eval"
    echo "+ python3 -m venv .venv-eval" >&2
    python3 -m venv "${ROOT}/.venv-eval"
    echo "+ .venv-eval/bin/pip install --require-hashes -r requirements-eval.lock" >&2
    "${ROOT}/.venv-eval/bin/pip" install -q --require-hashes -r "${ROOT}/requirements-eval.lock"
    printf '%s\n' "${fingerprint}" > "${marker}"
  fi
  echo "${ROOT}/.venv-eval/bin/python"
}

require_tool ruff
if [ "$FAST_GATE" -eq 0 ]; then
  require_tool mdbook
  require_tool cargo-deny
fi
run python3 scripts/lock_python_deps.py --check
EVAL_PY="$(eval_python)"
run python3 scripts/validate_workspace_deps.py
run ruff check scripts .github/actions/costguard/scripts
run cargo fmt --check
run cargo clippy --locked --all-targets --all-features -- -D warnings
if [ "$FAST_GATE" -eq 0 ]; then
  RUSTDOCFLAGS="-D warnings" run cargo doc --workspace --no-deps --locked
fi
run cargo build --release --locked -p costguard-cli
run python3 scripts/verify_release_assets.py
run "$EVAL_PY" -m unittest discover -s scripts/tests -p 'test_*.py'
if [ "$FAST_GATE" -eq 0 ]; then
  run python3 scripts/validate_fp_registry.py
  run python3 scripts/recall_report.py
  run "$EVAL_PY" scripts/eval_metrics.py --split corpus
  run "$EVAL_PY" scripts/eval_irr.py --check
  COSTGUARD_BUILD_PROFILE=release run python3 scripts/benchmark_external_repo.py --all-vendored
  run python3 scripts/generate_rule_docs.py --check
  run python3 scripts/generate_precision_tiers.py --check
  run python3 scripts/build_benchmark_evidence.py --check
  run python3 scripts/check_docs.py
  run mdbook build
  run cargo deny check
fi
run cargo test --workspace --all-targets --locked
if [ "$SPELLBOOK_SMOKE" -eq 1 ]; then
  COSTGUARD_BUILD_PROFILE=release run python3 scripts/benchmark_external_repo.py --repo spellbook --smoke
fi
if [ "$NBA_MONTE_CARLO_SMOKE" -eq 1 ]; then
  COSTGUARD_BUILD_PROFILE=release run python3 scripts/benchmark_external_repo.py --repo nba-monte-carlo --smoke
fi
if [ "$PRECISION_GATE" -eq 1 ]; then
  SPELLBOOK_CACHE="${HOME}/.cache/costguard/benchmarks/spellbook"
  if [ -f "${SPELLBOOK_CACHE}/target/manifest.json" ]; then
    run python3 scripts/precision_triage.py --repo spellbook --sample-size 200
    EVAL_PY="${EVAL_PY:-$(eval_python)}"
    run "$EVAL_PY" scripts/eval_metrics.py --split real
  else
    echo "ERROR: spellbook cache missing; precision gate cannot run" >&2
    exit 2
  fi
fi
if [ "$CENSUS_GATE" -eq 1 ]; then
  CACHE_ROOT="${HOME}/.cache/costguard/benchmarks"
  if [ -f "${CACHE_ROOT}/spellbook/target/manifest.json" ] \
    && [ -f "${CACHE_ROOT}/jaffle-shop/target/manifest.json" ]; then
    run python3 scripts/rule_tp_census.py --emit-evidence
  else
    echo "ERROR: benchmark cache missing; census gate cannot run" >&2
    exit 2
  fi
fi
