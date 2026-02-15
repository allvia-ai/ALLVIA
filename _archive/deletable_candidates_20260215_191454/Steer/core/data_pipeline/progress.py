"""Shared progress-bar state for the data pipeline.

This module centralizes the optional `tqdm` dependency and allows callers to
toggle progress-bar usage (e.g., `run_step2(use_tqdm=False)`) in a way that is
visible across submodules.
"""

from __future__ import annotations

from typing import Optional

try:  # pragma: no cover - optional dependency
    from tqdm import tqdm as _tqdm_impl
except Exception:  # pragma: no cover - optional dependency
    _tqdm_impl = None


def set_tqdm_enabled(enabled: bool) -> None:
    """Enable/disable tqdm usage across the data pipeline."""

    global tqdm
    tqdm = _tqdm_impl if enabled else None


def tqdm_write(message: str) -> None:
    """Best-effort write through tqdm when available."""

    if _tqdm_impl is not None:
        _tqdm_impl.write(message)
        return
    print(message)


tqdm: Optional[object] = _tqdm_impl

