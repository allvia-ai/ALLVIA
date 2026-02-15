#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

PY="${PYTHON:-python}"
if ! command -v "$PY" >/dev/null 2>&1; then
  PY="python3"
fi

exec "$PY" "$ROOT_DIR/scripts/search_api_server.py" "$@"

