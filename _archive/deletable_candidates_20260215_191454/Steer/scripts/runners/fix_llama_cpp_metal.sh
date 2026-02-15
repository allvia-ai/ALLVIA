#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PY="$REPO_ROOT/conda/envs/ai-summary/bin/python"

if [ ! -x "$PY" ]; then
  echo "❌ conda python not found: $PY" >&2
  exit 1
fi

VENDOR_SRC="$REPO_ROOT/llama_build/llama-cpp-python/vendor/llama.cpp"
if [ ! -d "$VENDOR_SRC" ]; then
  echo "❌ vendor llama.cpp not found: $VENDOR_SRC" >&2
  exit 1
fi

PKG_LIB="$("$PY" -c "from pathlib import Path; import llama_cpp; print(Path(llama_cpp.__file__).resolve().parent / 'lib')")"
if [ ! -d "$PKG_LIB" ]; then
  echo "❌ llama_cpp lib dir not found: $PKG_LIB" >&2
  exit 1
fi

BUILD_DIR="$VENDOR_SRC/build_ai_summary_metal"
echo "🔧 building llama.cpp shared libs (Metal)…"
rm -rf "$BUILD_DIR"
cmake -S "$VENDOR_SRC" -B "$BUILD_DIR" \
  -DCMAKE_BUILD_TYPE=Release \
  -DBUILD_SHARED_LIBS=ON \
  -DGGML_METAL=ON \
  -DGGML_OPENMP=OFF \
  -DGGML_ACCELERATE=ON \
  -DLLAMA_BUILD_TESTS=OFF \
  -DLLAMA_BUILD_EXAMPLES=OFF \
  >/dev/null
cmake --build "$BUILD_DIR" -j 4 >/dev/null

SRC_LIB="$BUILD_DIR/bin"
for f in libllama.dylib libggml.dylib libggml-base.dylib libggml-cpu.dylib libggml-blas.dylib libggml-metal.dylib libmtmd.dylib; do
  if [ ! -f "$SRC_LIB/$f" ]; then
    echo "❌ build output missing: $SRC_LIB/$f" >&2
    exit 1
  fi
done

TS="$(date +%Y%m%d-%H%M%S)"
BK="$PKG_LIB.bak.$TS"
mkdir -p "$BK"
cp -a "$PKG_LIB"/* "$BK"/

echo "🧩 installing rebuilt Metal dylibs into llama_cpp package…"
rm -f "$PKG_LIB"/libllama*.dylib "$PKG_LIB"/libggml*.dylib "$PKG_LIB"/libmtmd*.dylib || true
for f in libllama.dylib libggml.dylib libggml-base.dylib libggml-cpu.dylib libggml-blas.dylib libggml-metal.dylib libmtmd.dylib; do
  cp -a "$SRC_LIB/$f" "$PKG_LIB/"
done

echo "🧪 smoke test llama_cpp Metal (n_gpu_layers=1)…"
set +e
"$PY" - <<'PY'
from pathlib import Path
from llama_cpp import Llama

model = Path("models/gguf/tinyllama-1.1b-chat-v1.0.Q4_K_M.gguf").resolve()
llm = Llama(model_path=str(model), n_ctx=256, n_threads=4, n_gpu_layers=1, verbose=False)
out = llm("Q: 2+2?\nA:", max_tokens=8, temperature=0)
print(out["choices"][0]["text"].strip())
PY
RC=$?
set -e

if [ "$RC" -ne 0 ]; then
  echo "❌ llama_cpp Metal test failed; restoring backup: $BK" >&2
  rm -f "$PKG_LIB"/*
  cp -a "$BK"/* "$PKG_LIB"/
  exit 1
fi

echo "✅ llama_cpp Metal installed (backup: $BK)"

