#!/usr/bin/env bash
set -eo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"

if ! git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
  echo "costguard pre-commit: not a git repository; skipping"
  exit 0
fi

BASE="${COSTGUARD_BASE:-HEAD~1}"
WAREHOUSE="${COSTGUARD_WAREHOUSE:-snowflake}"
FAIL_ON="${COSTGUARD_FAIL_ON:-high}"
MIN_CONFIDENCE="${COSTGUARD_MIN_CONFIDENCE:-high}"

if command -v costguard >/dev/null 2>&1; then
  COSTGUARD_BIN="costguard"
elif [ -x "$ROOT/target/release/costguard" ]; then
  COSTGUARD_BIN="$ROOT/target/release/costguard"
elif [ -x "$ROOT/target/debug/costguard" ]; then
  COSTGUARD_BIN="$ROOT/target/debug/costguard"
else
  echo "costguard pre-commit: costguard binary not found in PATH or target/{release,debug}" >&2
  exit 1
fi

args=(
  pr
  --base "$BASE"
  --warehouse "$WAREHOUSE"
  --fail-on "$FAIL_ON"
  --min-confidence "$MIN_CONFIDENCE"
)

exec "$COSTGUARD_BIN" "${args[@]}"
