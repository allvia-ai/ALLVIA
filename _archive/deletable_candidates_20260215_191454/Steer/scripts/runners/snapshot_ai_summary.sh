#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

WITH_ENV=0
WITH_DATA=0
WITH_MODELS=0
WITH_LLAMA_BUILD=0

usage() {
  cat <<'EOF'
Usage:
  ./scripts/snapshot_ai_summary.sh [--with-env] [--with-data] [--with-models]

Creates a timestamped snapshot under ./backups/.

Defaults (safe + smaller):
  - includes repo code/config
  - excludes heavy/generated folders (artifacts/, data/cache/, __pycache__/ etc)

Options:
  --with-env   include ./conda/envs/ai-summary (can be very large)
  --with-data  include data/ and artifacts/ (can be large)
  --with-models include models/gguf and models/llama.cpp (very large)
  --with-llama-build include ./llama_build (large)
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    -h|--help) usage; exit 0 ;;
    --with-env) WITH_ENV=1; shift ;;
    --with-data) WITH_DATA=1; shift ;;
    --with-models) WITH_MODELS=1; shift ;;
    --with-llama-build) WITH_LLAMA_BUILD=1; shift ;;
    *) echo "unknown arg: $1" >&2; usage; exit 2 ;;
  esac
done

TS="$(date +%Y%m%d-%H%M%S)"
OUT_DIR="$REPO_ROOT/backups"
OUT_TAR="$OUT_DIR/AI-summary-snapshot-$TS.tar.gz"
OUT_TAR_TMP="$OUT_DIR/.tmp-AI-summary-snapshot-$TS.tar.gz"
META_DIR="$OUT_DIR/AI-summary-snapshot-$TS.meta"

mkdir -p "$OUT_DIR" "$META_DIR"

PY="$REPO_ROOT/conda/envs/ai-summary/bin/python"

cleanup() {
  rm -f "$OUT_TAR_TMP" >/dev/null 2>&1 || true
}
trap cleanup EXIT

{
  echo "snapshot_ts=$TS"
  echo "repo_root=$REPO_ROOT"
  echo "uname=$(uname -a)"
  if command -v git >/dev/null 2>&1; then
    (cd "$REPO_ROOT" && echo "git_head=$(git rev-parse HEAD 2>/dev/null || true)")
    (cd "$REPO_ROOT" && echo "git_status=$(git status --porcelain 2>/dev/null | wc -l | tr -d ' ')")
  fi
} > "$META_DIR/meta.txt"

if [[ -x "$PY" ]]; then
  "$PY" -m pip freeze > "$META_DIR/pip_freeze.txt" || true
  "$PY" -c "import sys; print(sys.version)" > "$META_DIR/python_version.txt" || true
else
  echo "warn: conda python not found at $PY" > "$META_DIR/warn.txt"
fi

EXCLUDES=(
  "--exclude=./backups"
  "--exclude=./.git"
  "--exclude=./__pycache__"
  "--exclude=./.pytest_cache"
  "--exclude=./.pycache_tmp"
  "--exclude=./**/__pycache__"
  "--exclude=./**/.DS_Store"
)

if [[ "$WITH_ENV" -eq 0 ]]; then
  EXCLUDES+=("--exclude=./conda")
else
  # Keep only the named env; exclude other conda payloads if present.
  EXCLUDES+=("--exclude=./conda/pkgs" "--exclude=./conda/condabin")
fi

if [[ "$WITH_DATA" -eq 0 ]]; then
  EXCLUDES+=(
    "--exclude=./data"
    "--exclude=./artifacts"
  )
fi

if [[ "$WITH_MODELS" -eq 0 ]]; then
  EXCLUDES+=(
    "--exclude=./models"
  )
fi

if [[ "$WITH_LLAMA_BUILD" -eq 0 ]]; then
  EXCLUDES+=(
    "--exclude=./llama_build"
  )
fi

tar -C "$REPO_ROOT" "${EXCLUDES[@]}" -czf "$OUT_TAR_TMP" .
mv -f "$OUT_TAR_TMP" "$OUT_TAR"
tar -C "$OUT_DIR" -czf "$OUT_DIR/AI-summary-snapshot-$TS.meta.tar.gz" "AI-summary-snapshot-$TS.meta"

echo "✅ snapshot created:"
echo "  - $OUT_TAR"
echo "  - $OUT_DIR/AI-summary-snapshot-$TS.meta.tar.gz"
echo
echo "Restore example:"
echo "  mkdir -p /tmp/AI-summary-restore && tar -C /tmp/AI-summary-restore -xzf \"$OUT_TAR\""
