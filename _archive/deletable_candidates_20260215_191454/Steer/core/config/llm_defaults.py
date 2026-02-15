"""Shared defaults for local LLM backend/model to keep UI/core in sync."""

from __future__ import annotations

import os
from typing import Optional

# Default values used across UI/CLI/document agents
DEFAULT_LLM_BACKEND = os.getenv("DEFAULT_LLM_BACKEND", "local_llamacpp").strip() or "local_llamacpp"
DEFAULT_LLM_MODEL = (
    os.getenv("DEFAULT_LLM_MODEL", "models/gguf/gemma-3-4b-it-Q4_K_M.gguf").strip()
    or "models/gguf/gemma-3-4b-it-Q4_K_M.gguf"
)


def resolve_backend(
    *,
    config_backend: Optional[str] = None,
    env_backend: Optional[str] = None,
    model: Optional[str] = None,
) -> str:
    """
    Decide backend when config/env are empty.
    - If any backend is provided, return it.
    - If model looks like a GGUF/local path, prefer local_llamacpp.
    - Otherwise fall back to DEFAULT_LLM_BACKEND.
    """
    backend = (config_backend or env_backend or "").strip()
    if backend:
        return backend

    model_val = (model or "").strip()
    if model_val.endswith(".gguf") or "/" in model_val:
        return "local_llamacpp"

    return DEFAULT_LLM_BACKEND
