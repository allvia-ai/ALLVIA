"""One-shot llama.cpp worker for conversational RAG.

Unlike the meeting summarisation worker, this:
- Does NOT chunk the input text
- Properly handles system prompts
- Returns {"response": "..."} format for chat use
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

try:
    from llama_cpp import Llama  # type: ignore
except Exception:  # pragma: no cover - optional dependency
    Llama = None


def _read_payload() -> dict:
    raw = sys.stdin.buffer.read()
    if not raw:
        return {}
    return json.loads(raw.decode("utf-8", "ignore"))


def main() -> None:
    payload = _read_payload()
    prompt = str(payload.get("prompt") or "").strip()
    system = str(payload.get("system") or "").strip()
    model_path = str(payload.get("model_path") or "").strip()
    
    if not prompt or not model_path or Llama is None:
        print(json.dumps({"response": ""}, ensure_ascii=False))
        return

    model_file = Path(model_path).expanduser()
    if not model_file.exists():
        raise SystemExit(f"model not found: {model_file}")

    n_ctx = max(256, int(payload.get("n_ctx") or 4096))
    n_threads = int(payload.get("n_threads") or 0)
    n_gpu_layers = int(payload.get("n_gpu_layers") or 0)
    max_new_tokens = max(1, int(payload.get("max_new_tokens") or 512))
    temperature = float(payload.get("temperature") or 0.7)

    llm = Llama(
        model_path=str(model_file),
        n_ctx=n_ctx,
        n_threads=n_threads if n_threads > 0 else None,
        n_gpu_layers=n_gpu_layers,
        logits_all=False,
        verbose=False,
    )

    # Build full prompt with system instruction
    if system:
        full_prompt = f"{system}\n\n{prompt}"
    else:
        full_prompt = prompt

    out = llm(
        prompt=full_prompt,
        max_tokens=max_new_tokens,
        temperature=temperature,
        stop=["</s>", "<|end|>", "<end_of_turn>"],
    )
    
    text = ""
    if isinstance(out, dict):
        choices = out.get("choices") or []
        if choices and isinstance(choices[0], dict):
            text = choices[0].get("text", "") or ""
    if not text:
        text = str(out)

    print(json.dumps({"response": text.strip()}, ensure_ascii=False))


if __name__ == "__main__":
    main()
