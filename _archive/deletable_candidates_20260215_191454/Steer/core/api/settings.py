"""Configuration object for the lightweight FastAPI backend used in tests."""
from __future__ import annotations

import os
from dataclasses import dataclass
from typing import Optional


@dataclass
class Settings:
    """Thin settings wrapper that mirrors the previous backend defaults."""

    TESTING: bool = False
    STARTUP_LOAD: bool = True
    LLM_ENABLED: bool = False
    LLM_PROVIDER: str = "none"
    LLM_MODEL: Optional[str] = None
    LLM_API_KEY: Optional[str] = None
    LLM_BASE_URL: Optional[str] = None
    LLM_CONTEXT_DOCS: int = 4
    LLM_TEMPERATURE: float = 0.2
    LLM_LANGUAGE: str = "ko"

    def __init__(
        self,
        TESTING: bool | None = None,
        STARTUP_LOAD: bool | None = None,
        LLM_ENABLED: bool | None = None,
        LLM_PROVIDER: Optional[str] = None,
        LLM_MODEL: Optional[str] = None,
        LLM_API_KEY: Optional[str] = None,
        LLM_BASE_URL: Optional[str] = None,
        LLM_CONTEXT_DOCS: Optional[int] = None,
        LLM_TEMPERATURE: Optional[float] = None,
        LLM_LANGUAGE: Optional[str] = None,
    ) -> None:
        if TESTING is not None:
            self.TESTING = bool(TESTING)
        else:
            self.TESTING = os.getenv("APP_TESTING", "0") == "1"

        if STARTUP_LOAD is not None:
            self.STARTUP_LOAD = bool(STARTUP_LOAD)
        else:
            self.STARTUP_LOAD = os.getenv("APP_STARTUP_LOAD", "1") != "0"

        provider_env = os.getenv("LLM_PROVIDER", "none")
        provider = (LLM_PROVIDER or provider_env or "none").strip() or "none"
        self.LLM_PROVIDER = provider

        if LLM_ENABLED is not None:
            self.LLM_ENABLED = bool(LLM_ENABLED)
        else:
            enabled_env = os.getenv("LLM_ENABLED")
            if enabled_env is not None:
                self.LLM_ENABLED = enabled_env.strip().lower() not in {"", "0", "false"}
            else:
                self.LLM_ENABLED = provider.lower() != "none"

        self.LLM_MODEL = LLM_MODEL or os.getenv("LLM_MODEL") or None
        self.LLM_API_KEY = LLM_API_KEY or os.getenv("LLM_API_KEY") or None
        self.LLM_BASE_URL = LLM_BASE_URL or os.getenv("LLM_BASE_URL") or None

        context_env = LLM_CONTEXT_DOCS if LLM_CONTEXT_DOCS is not None else os.getenv("LLM_CONTEXT_DOCS")
        try:
            self.LLM_CONTEXT_DOCS = int(context_env) if context_env is not None else 4
        except ValueError:
            self.LLM_CONTEXT_DOCS = 4

        temperature_env = (
            LLM_TEMPERATURE if LLM_TEMPERATURE is not None else os.getenv("LLM_TEMPERATURE")
        )
        try:
            self.LLM_TEMPERATURE = float(temperature_env) if temperature_env is not None else 0.2
        except ValueError:
            self.LLM_TEMPERATURE = 0.2

        language_env = LLM_LANGUAGE or os.getenv("LLM_LANGUAGE")
        self.LLM_LANGUAGE = (language_env or "ko").strip() or "ko"
