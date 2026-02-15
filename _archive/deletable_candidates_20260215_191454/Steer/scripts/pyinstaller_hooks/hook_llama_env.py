"""
Runtime hook for PyInstaller: set LLAMA_CPP_LIB and default GGUF path at startup.

This lets packaged binaries find the bundled libllama.dylib and Gemma3 model
without manual environment setup.
"""
from __future__ import annotations

import os
import sys
from pathlib import Path


def _set_env() -> None:
    base = Path(getattr(sys, "_MEIPASS", Path(__file__).resolve().parent))
    # Ensure bundled sources (scripts/, etc.) are importable
    sys.path.insert(0, str(base))
    scripts_dir = base / "scripts"
    if scripts_dir.exists():
        sys.path.insert(0, str(scripts_dir.parent))

    lib = base / "libllama.dylib"
    if lib.exists():
        os.environ.setdefault("LLAMA_CPP_LIB", str(lib))

    model = base / "models" / "gguf" / "gemma-3-4b-it-Q4_K_M.gguf"
    if model.exists():
        os.environ.setdefault("MEETING_SUMMARY_LLAMA_MODEL", str(model))
        os.environ.setdefault("LNPCHAT_LLM_MODEL", str(model))


_set_env()
