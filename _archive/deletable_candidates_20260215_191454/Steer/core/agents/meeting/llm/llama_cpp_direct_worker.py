"""One-shot llama-cpp-python worker for meeting summarisation.

Runs in a subprocess to isolate potential native crashes from the main process.
Reads a JSON payload from stdin and prints {"summary": "..."} to stdout.
"""

from __future__ import annotations

import json
import sys
from pathlib import Path
from typing import List

try:
    from llama_cpp import Llama  # type: ignore
except Exception:  # pragma: no cover - optional dependency
    Llama = None


def _chunk_text(text: str, limit: int) -> List[str]:
    if limit <= 0:
        return [text]
    text = (text or "").strip()
    if not text:
        return []
    chunks: List[str] = []
    start = 0
    while start < len(text):
        end = min(len(text), start + limit)
        chunks.append(text[start:end])
        start = end
    return chunks


def _read_payload() -> dict:
    raw = sys.stdin.buffer.read()
    if not raw:
        return {}
    return json.loads(raw.decode("utf-8", "ignore"))


def main() -> None:
    payload = _read_payload()
    transcript = str(payload.get("transcript") or "")
    model_path = str(payload.get("model_path") or "").strip()
    if not transcript or not model_path or Llama is None:
        print(json.dumps({"summary": ""}, ensure_ascii=False))
        return

    model_file = Path(model_path).expanduser()
    if not model_file.exists():
        raise SystemExit(f"model not found: {model_file}")

    n_ctx = max(256, int(payload.get("n_ctx") or 4096))
    n_threads = int(payload.get("n_threads") or 0)
    n_gpu_layers = int(payload.get("n_gpu_layers") or 0)
    max_new_tokens = max(1, int(payload.get("max_new_tokens") or 256))
    chunk_char_limit = max(256, int(payload.get("chunk_char_limit") or 1800))
    prompt_template = str(payload.get("prompt_template") or "{transcript}")

    llm = Llama(
        model_path=str(model_file),
        n_ctx=n_ctx,
        n_threads=n_threads if n_threads > 0 else None,
        n_gpu_layers=n_gpu_layers,
        logits_all=False,
        verbose=False,
    )

    def _summarise_once(chunk: str) -> str:
        prompt = prompt_template.format(transcript=chunk)
        out = llm(
            prompt=prompt,
            max_tokens=max_new_tokens,
            temperature=0.0,
            stop=["</s>"],
        )
        text = ""
        if isinstance(out, dict):
            choices = out.get("choices") or []
            if choices and isinstance(choices[0], dict):
                text = choices[0].get("text", "") or ""
        if not text:
            text = str(out)
        return text.strip()

    chunks = _chunk_text(transcript, chunk_char_limit)
    parts = [_summarise_once(chunk) for chunk in chunks if chunk.strip()]
    parts = [p for p in parts if p]
    if not parts:
        print(json.dumps({"summary": ""}, ensure_ascii=False))
        return
    if len(parts) == 1:
        summary = parts[0]
    else:
        summary = _summarise_once(" ".join(parts)) or " ".join(parts)

    print(json.dumps({"summary": summary}, ensure_ascii=False))


if __name__ == "__main__":
    main()

