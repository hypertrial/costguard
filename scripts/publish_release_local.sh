#!/usr/bin/env bash
set -eo pipefail
exec python3 "$(dirname "$0")/publish_release_local.py" "$@"
