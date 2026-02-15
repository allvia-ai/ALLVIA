"""Utility to verify local LLM availability (Ollama, llama.cpp)."""
from __future__ import annotations

import argparse
import shutil
import subprocess
import sys


def check_ollama(model: str | None) -> int:
    if shutil.which("ollama") is None:
        print("ollama executable not found in PATH", file=sys.stderr)
        return 1
    try:
        subprocess.run(["ollama", "--version"], check=False, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    except OSError as exc:
        print(f"failed to invoke ollama: {exc}", file=sys.stderr)
        return 1

    if model:
        result = subprocess.run(["ollama", "list"], stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, check=False)
        if result.returncode != 0:
            print(result.stderr.strip() or "ollama list failed", file=sys.stderr)
            return 1
        if model not in result.stdout:
            print(f"model '{model}' not found. install with: ollama pull {model}", file=sys.stderr)
            return 1
        print(f"ollama ready. model '{model}' installed.")
    else:
        print("ollama ready.")
    return 0


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("backend", choices=["ollama"], help="LLM backend to verify")
    parser.add_argument("--model", help="Model name to check (ollama)")
    args = parser.parse_args()

    if args.backend == "ollama":
        sys.exit(check_ollama(args.model))


if __name__ == "__main__":  # pragma: no cover - CLI entry point
    main()
