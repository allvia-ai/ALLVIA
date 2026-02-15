#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PY="$REPO_ROOT/conda/envs/ai-summary/bin/python"

exec "$PY" "$REPO_ROOT/scripts/run_photo_agent.py" "$@"

