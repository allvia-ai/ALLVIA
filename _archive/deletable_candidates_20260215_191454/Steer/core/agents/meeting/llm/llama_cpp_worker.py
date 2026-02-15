"""One-shot llama.cpp worker for meeting summarisation.

Runs in a subprocess to isolate potential native crashes from the main process.
Uses the bundled `models/llama.cpp/build_cpu/bin/llama-cli` when available.
Reads a JSON payload from stdin and prints {"summary": "..."} to stdout.
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import List, Optional


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
    raw = sys.stdin.read()
    if not raw:
        return {}
    return json.loads(raw)


def main() -> None:
    payload = _read_payload()
    transcript = str(payload.get("transcript") or "")
    model_path = str(payload.get("model_path") or "").strip()
    if not transcript or not model_path:
        print(json.dumps({"summary": ""}, ensure_ascii=False))
        return

    model_file = Path(model_path).expanduser()
    if not model_file.exists():
        raise SystemExit(f"model not found: {model_file}")

    n_ctx = max(256, int(payload.get("n_ctx") or 4096))
    n_threads = int(payload.get("n_threads") or 0)
    max_new_tokens = max(1, int(payload.get("max_new_tokens") or 256))
    chunk_char_limit = max(256, int(payload.get("chunk_char_limit") or 1800))
    prompt_template = str(payload.get("prompt_template") or "{transcript}")
    gpu_layers = int(payload.get("gpu_layers") or payload.get("n_gpu_layers") or 0)

    repo_root = Path(__file__).resolve().parents[4]
    override = (str(payload.get("cli_path") or "") or os.getenv("MEETING_LLAMA_CLI_PATH") or "").strip()
    candidates: List[Path] = []
    if override:
        candidates.append(Path(override).expanduser())
    candidates.append(repo_root / "models" / "llama.cpp" / "build_metal" / "bin" / "llama-cli")
    candidates.append(repo_root / "models" / "llama.cpp" / "build_cpu" / "bin" / "llama-cli")
    candidates = [p for p in candidates if p.exists()]
    if not candidates:
        tried = [Path(override).expanduser()] if override else []
        tried.extend(
            [
                repo_root / "models" / "llama.cpp" / "build_metal" / "bin" / "llama-cli",
                repo_root / "models" / "llama.cpp" / "build_cpu" / "bin" / "llama-cli",
            ]
        )
        raise SystemExit(f"llama-cli not found (tried: {', '.join(str(p) for p in tried)})")

    no_mmap_env = os.getenv("MEETING_LLAMA_CLI_NO_MMAP", "").strip().lower()
    no_mmap_override = None
    if no_mmap_env in {"1", "true", "yes"}:
        no_mmap_override = True
    elif no_mmap_env in {"0", "false", "no"}:
        no_mmap_override = False

    def _summarise_once(chunk: str) -> str:
        prompt = prompt_template.format(transcript=chunk)
        last_error = ""
        text = ""
        for llama_cli in candidates:
            no_mmap = no_mmap_override if no_mmap_override is not None else ("build_metal" in str(llama_cli))
            cmd = [
                str(llama_cli),
                "--simple-io",
                "--no-display-prompt",
                "--no-perf",
                "-m",
                str(model_file),
                "-c",
                str(n_ctx),
                "-n",
                str(max_new_tokens),
                "--temp",
                "0",
                "-ngl",
                str(gpu_layers),
                "-p",
                prompt,
            ]
            if no_mmap:
                cmd.append("--no-mmap")
            if n_threads > 0:
                cmd.extend(["-t", str(n_threads)])

            env = os.environ.copy()
            env.setdefault("LLAMA_ARG_NO_PERF", "1")
            proc = subprocess.run(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=env)
            if proc.returncode == 0:
                text = proc.stdout.decode("utf-8", "ignore").strip()
                break
            err = proc.stderr.decode("utf-8", "ignore").strip()
            last_error = err or f"llama-cli exited {proc.returncode}"
            if override:
                break
        if not text:
            raise SystemExit(f"llama-cli failed: {last_error[:300]}")

        if not text:
            return ""
        lines = []
        for line in text.splitlines():
            trimmed = line.strip()
            if "EOF by user" in trimmed or trimmed.endswith("EOF by user"):
                continue
            if trimmed.startswith(">") and "EOF" in trimmed:
                continue
            lines.append(line)
        return "\n".join(lines).strip()

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
