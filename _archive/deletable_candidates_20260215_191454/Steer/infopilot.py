"""Compatibility shim for the original ``infopilot`` module location."""

import os as _os
from importlib import import_module as _import_module
import sys as _sys
from pathlib import Path as _Path

_os.environ.setdefault("KMP_DUPLICATE_LIB_OK", "TRUE")
_os.environ.setdefault("OMP_NUM_THREADS", "1")
_os.environ.setdefault("PYTORCH_ENABLE_MPS_FALLBACK", "1")
_os.environ.setdefault("KMP_AFFINITY", "disabled")
_os.environ.setdefault("KMP_BLOCKTIME", "0")
if (_os.getenv("INFOPILOT_DEBUG_SHIM") or "").strip().lower() in {"1", "true", "yes", "on"}:
    _os.environ.setdefault("KMP_SETTINGS", "1")


def _debug_runtime_env() -> None:
    # Helps diagnose PyInstaller runtime layout
    print(">>> cwd =", _os.getcwd(), flush=True)
    print(">>> sys.path =", _sys.path, flush=True)
    try:
        print(">>> files =", _os.listdir("."), flush=True)
    except Exception as exc:  # pragma: no cover - defensive
        print(">>> listdir failed:", exc, flush=True)
    base = _Path(getattr(_sys, "_MEIPASS", _Path(__file__).resolve().parent))
    print(">>> _MEIPASS base =", base, flush=True)
    if base.exists():
        try:
            print(">>> _MEIPASS files =", [p.name for p in base.iterdir()], flush=True)
        except Exception as exc:  # pragma: no cover - defensive
            print(">>> _MEIPASS list failed:", exc, flush=True)

if (_os.getenv("INFOPILOT_DEBUG_SHIM") or "").strip().lower() in {"1", "true", "yes", "on"}:
    _debug_runtime_env()

_impl = _import_module("scripts.pipeline.infopilot")
_sys.modules[__name__] = _impl

if __name__ == "__main__":
    if hasattr(_impl, "main"):
        _impl.main()  # type: ignore[attr-defined]
    else:
        _impl.cli(obj={})  # type: ignore[attr-defined]
