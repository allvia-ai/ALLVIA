#!/usr/bin/env bash
# Package the CLI (infopilot.py) with local llama backend and bundled GGUF/libllama.
# Requirements: pyinstaller installed in current env.
set -euo pipefail

ROOT="$(cd -- "$(dirname "$0")/.." && pwd)"
APP_ENTRY="${ROOT}/infopilot.py"
LIB_LLAMA="${ROOT}/llama_build/llama.cpp/build/bin/libllama.dylib"
MODEL_GGUF="${ROOT}/models/gguf/gemma-3-4b-it-Q4_K_M.gguf"
HOOK_DIR="${ROOT}/scripts/pyinstaller_hooks"

if [ ! -f "$LIB_LLAMA" ]; then
  echo "[ERROR] libllama not found at $LIB_LLAMA" >&2
  exit 1
fi
if [ ! -f "$MODEL_GGUF" ]; then
  echo "[ERROR] GGUF not found at $MODEL_GGUF" >&2
  exit 1
fi

echo "[INFO] Building one-folder bundle with PyInstaller"
pyinstaller --clean --noconfirm \
  --name ai-summary-cli \
  --additional-hooks-dir "$HOOK_DIR" \
  --paths "$ROOT" \
  --hidden-import queue \
  --hidden-import click \
  --hidden-import scripts.pipeline.infopilot \
  --collect-submodules scripts \
  --add-binary "$LIB_LLAMA:." \
  --add-data "$MODEL_GGUF:models/gguf" \
  "$APP_ENTRY"

echo "[INFO] Done. Bundle at dist/ai-summary-cli"
