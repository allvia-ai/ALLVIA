#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PY="$REPO_ROOT/conda/envs/ai-summary/bin/python"

cd "$REPO_ROOT"
if [[ -f "$REPO_ROOT/.env" ]]; then
  set -a
  # shellcheck disable=SC1091
  source "$REPO_ROOT/.env"
  set +a
fi
exec "$PY" "$REPO_ROOT/scripts/pipeline/infopilot.py" --no-mlflow chat --query "/photo" "$@"
