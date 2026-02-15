# core/config/env.py
"""
Centralized environment variable configuration.
All `os.getenv` calls should ideally flow through here or be typed here.
"""
from __future__ import annotations

import os
from typing import Optional

def get_env_bool(key: str, default: bool = False) -> bool:
    val = os.getenv(key, str(default)).lower().strip()
    return val in {"true", "1", "yes", "on"}

def get_env_int(key: str, default: int = 0) -> int:
    try:
        return int(os.getenv(key, str(default)))
    except ValueError:
        return default

def get_env_float(key: str, default: float = 0.0) -> float:
    try:
        return float(os.getenv(key, str(default)))
    except ValueError:
        return default

def get_env_str(key: str, default: str = "") -> str:
    return os.getenv(key, default).strip()

# --- Feature Flags ---
ENABLE_MEETING_AGENT = get_env_bool("INFOPILOT_ENABLE_MEETING", True)
ENABLE_PHOTO_AGENT = get_env_bool("INFOPILOT_ENABLE_PHOTO", True)
ENABLE_WEB_AGENT = get_env_bool("INFOPILOT_ENABLE_WEB", False)

# --- LLM defaults (Aggregated from llm_defaults.py if we move them) ---
DEFAULT_LLM_MODEL = get_env_str("INFOPILOT_LLM_MODEL", "google/gemma-2-9b-it") 
# Previously hardcoded or in llm_defaults

# --- Path Overrides ---
INFOPILOT_DATA_DIR_OVERRIDE: Optional[str] = os.getenv("INFOPILOT_DATA_DIR")
