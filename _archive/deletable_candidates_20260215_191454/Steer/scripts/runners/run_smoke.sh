#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PY="$REPO_ROOT/conda/envs/ai-summary/bin/python"

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
  cat <<'EOF'
Usage:
  ./scripts/run_smoke.sh

Checks that core artifacts exist and that doc/meeting/photo agents respond in non-interactive mode.
EOF
  exit 0
fi

cd "$REPO_ROOT"

missing=0
for f in "$REPO_ROOT/data/corpus.parquet" "$REPO_ROOT/data/topic_model.joblib"; do
  if [[ ! -f "$f" ]]; then
    echo "❌ missing: $f" >&2
    missing=1
  fi
done
if [[ ! -d "$REPO_ROOT/data/cache" ]]; then
  echo "❌ missing: $REPO_ROOT/data/cache" >&2
  missing=1
fi
if [[ "$missing" -ne 0 ]]; then
  echo "Run ./scripts/run_refresh.sh first." >&2
  exit 2
fi

echo "✅ artifacts ok"

doc_payload="$("$PY" "$REPO_ROOT/scripts/pipeline/infopilot.py" --no-mlflow chat --scope global --no-rerank --json --query "/search AI-summary" 2>/dev/null || true)"
if [[ -z "$doc_payload" ]]; then
  echo "❌ document smoke failed" >&2
  exit 3
fi
echo "✅ document ok"

photo_payload="$("$PY" "$REPO_ROOT/scripts/pipeline/infopilot.py" --no-mlflow chat --scope global --no-rerank --json --query "/photo" 2>/dev/null || true)"
if [[ -z "$photo_payload" ]]; then
  echo "❌ photo smoke failed" >&2
  exit 4
fi
echo "✅ photo ok"

meeting_payload="$("$PY" "$REPO_ROOT/scripts/pipeline/infopilot.py" --no-mlflow chat --scope global --no-rerank --json --query "/meeting" 2>/dev/null || true)"
if [[ -z "$meeting_payload" ]]; then
  echo "❌ meeting smoke failed" >&2
  exit 5
fi
echo "✅ meeting ok"

echo "🎉 smoke ok"
