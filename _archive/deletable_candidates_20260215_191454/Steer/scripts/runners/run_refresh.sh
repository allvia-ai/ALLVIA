#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
  cat <<'EOF'
Usage:
  ./scripts/run_refresh.sh

Runs incremental-friendly: scan(--hash) -> train -> index using repo-local conda python when available.

Env overrides:
  SCAN_CSV, CORPUS, MODEL, CACHE_DIR, POLICY, EMBED_BATCH_SIZE
EOF
  exit 0
fi

cd "$REPO_ROOT"

PY="$REPO_ROOT/conda/envs/ai-summary/bin/python"
if [[ ! -x "$PY" ]]; then
  if command -v python3 >/dev/null 2>&1; then
    PY="$(command -v python3)"
  else
    echo "python executable not found (expected: $REPO_ROOT/conda/envs/ai-summary/bin/python)" >&2
    exit 1
  fi
fi

SCAN_CSV="${SCAN_CSV:-$REPO_ROOT/data/found_files.csv}"
CORPUS="${CORPUS:-$REPO_ROOT/data/corpus.parquet}"
MODEL="${MODEL:-$REPO_ROOT/data/topic_model.joblib}"
CACHE_DIR="${CACHE_DIR:-$REPO_ROOT/data/cache}"
POLICY="${POLICY:-$REPO_ROOT/core/config/smart_folders.json}"

EMBED_BATCH_SIZE="${EMBED_BATCH_SIZE:-16}"

"$PY" "$REPO_ROOT/scripts/pipeline/infopilot.py" --no-mlflow scan \
  --policy "$POLICY" --out "$SCAN_CSV" --hash

"$PY" "$REPO_ROOT/scripts/pipeline/infopilot.py" --no-mlflow train \
  --scan-csv "$SCAN_CSV" --use-embedding --no-async-embed --embedding-batch-size "$EMBED_BATCH_SIZE"

"$PY" "$REPO_ROOT/scripts/pipeline/infopilot.py" --no-mlflow index \
  --scope global --corpus "$CORPUS" --model "$MODEL" --cache "$CACHE_DIR"

echo "✅ refresh complete"
