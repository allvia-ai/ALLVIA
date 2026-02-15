#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PY="$REPO_ROOT/conda/envs/ai-summary/bin/python"

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
  cat <<'EOF'
Usage:
  ./scripts/setup_meeting_llama.sh /absolute/path/to/model.gguf

Writes/updates repo .env so meeting summarizer uses llama.cpp (subprocess mode by default).
EOF
  exit 0
fi

MODEL_PATH="${1:-}"
if [[ -z "$MODEL_PATH" ]]; then
  echo "model path required (gguf). Example: ./scripts/setup_meeting_llama.sh /path/to/model.gguf" >&2
  exit 2
fi

if [[ ! -f "$MODEL_PATH" ]]; then
  echo "model file not found: $MODEL_PATH" >&2
  exit 3
fi

cd "$REPO_ROOT"

"$PY" - <<PY
from __future__ import annotations

from pathlib import Path

env_path = Path(".env")
model_path = Path(r"""$MODEL_PATH""").expanduser().resolve()

updates = {
    "MEETING_SUMMARY_BACKEND": "llama",
    "MEETING_SUMMARY_LLAMA_MODEL": str(model_path),
    "MEETING_LLAMA_CPP_SUBPROCESS": "1",
    "MEETING_LLAMA_MODE": "auto",
    "MEETING_SUMMARY_LLAMA_GPU_LAYERS": "-1",
}

lines = []
existing = {}
if env_path.exists():
    for raw in env_path.read_text(encoding="utf-8").splitlines():
        line = raw.rstrip("\n")
        if not line.strip() or line.lstrip().startswith("#") or "=" not in line:
            lines.append(line)
            continue
        key, value = line.split("=", 1)
        key = key.strip()
        existing[key] = value
        if key in updates:
            lines.append(f"{key}={updates[key]}")
        else:
            lines.append(line)
else:
    lines.append("# Auto-generated meeting llama.cpp settings")

for key, value in updates.items():
    if key not in existing:
        lines.append(f"{key}={value}")

env_path.write_text("\n".join(lines).rstrip() + "\n", encoding="utf-8")
print(f"✅ updated {env_path} (MEETING_SUMMARY_BACKEND=llama, mode=auto, gpu_layers=-1)")
PY

echo "Next: ./scripts/run_meeting.sh"
