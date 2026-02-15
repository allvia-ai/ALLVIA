#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PY="$REPO_ROOT/conda/envs/ai-summary/bin/python"

if [ ! -x "$PY" ]; then
  echo "❌ conda python not found: $PY" >&2
  exit 1
fi

MODEL_CHAT="${LNPCHAT_LLM_MODEL:-$REPO_ROOT/models/gguf/gemma-3-4b-it-Q4_K_M.gguf}"
MODEL_MEETING="${MEETING_SUMMARY_LLAMA_MODEL:-$REPO_ROOT/models/gguf/gemma-3-4b-it-Q4_K_M.gguf}"
GPU_LAYERS_CHAT="${LNPCHAT_LLAMACPP_GPU_LAYERS:--1}"
GPU_LAYERS_MEETING="${MEETING_SUMMARY_LLAMA_GPU_LAYERS:--1}"

echo "== Mode#1 (llama-cpp-python direct) verification =="
echo "chat model:    $MODEL_CHAT"
echo "chat gpu:      $GPU_LAYERS_CHAT"
echo "meeting model: $MODEL_MEETING"
echo "meeting gpu:   $GPU_LAYERS_MEETING"
echo

set +e
echo "[1/2] LNPChat LLM (direct llama_cpp)…"
"$PY" - <<PY
from pathlib import Path
from llama_cpp import Llama

model = Path(r"""$MODEL_CHAT""").expanduser().resolve()
llm = Llama(model_path=str(model), n_ctx=256, n_threads=4, n_gpu_layers=int(r"""$GPU_LAYERS_CHAT"""), verbose=False)
out = llm("Q: 2+2?\\nA:", max_tokens=8, temperature=0)
print(out["choices"][0]["text"].strip()[:80])
PY
RC1=$?
if [ "$RC1" -eq 0 ]; then
  echo "✅ OK"
else
  echo "❌ FAIL (exit=$RC1)"
fi
echo

echo "[2/2] Meeting summariser (direct worker)…"
"$PY" - <<PY | "$PY" -m core.agents.meeting.llm.llama_cpp_direct_worker
import json
from pathlib import Path
model = str(Path(r"""$MODEL_MEETING""").expanduser().resolve())
payload = {
  "transcript": "[회의] 액션아이템: mode1 점검",
  "model_path": model,
  "n_ctx": 512,
  "n_threads": 4,
  "n_gpu_layers": int(r"""$GPU_LAYERS_MEETING"""),
  "max_new_tokens": 32,
  "chunk_char_limit": 1800,
  "prompt_template": "{transcript}\\n요약:",
}
print(json.dumps(payload, ensure_ascii=False))
PY
RC2=$?
if [ "$RC2" -eq 0 ]; then
  echo "✅ OK"
else
  echo "❌ FAIL (exit=$RC2)"
fi
set -e

echo
if [ "$RC1" -ne 0 ] || [ "$RC2" -ne 0 ]; then
  echo "⚠️ Mode#1이 실패하면, 설정상 자동으로 Mode#2/3(LLM fallback)로 내려가도록 되어 있습니다."
  exit 2
fi
echo "🎉 Mode#1 verified"

