"""Logging helpers for InfoPilot core modules."""
from __future__ import annotations

import logging
import os
from datetime import date
from pathlib import Path
from functools import lru_cache
from typing import Optional

from core.config.paths import RUNTIME_LOG_DIR

_DEFAULT_FORMAT = "%(asctime)s | %(levelname)s | %(name)s: %(message)s"


@lru_cache(maxsize=128)
def get_logger(name: Optional[str] = None) -> logging.Logger:
    """Return a module-level logger configured with the shared format."""
    logger = logging.getLogger(name or "infopilot")
    level_name = (os.getenv("INFOPILOT_LOG_LEVEL") or "INFO").strip().upper()
    level = getattr(logging, level_name, logging.INFO)
    logger.setLevel(level)

    formatter = logging.Formatter(_DEFAULT_FORMAT)

    if not any(isinstance(h, logging.StreamHandler) for h in logger.handlers):
        handler = logging.StreamHandler()
        handler.setLevel(level)
        handler.setFormatter(formatter)
        logger.addHandler(handler)

    runtime_dir = os.getenv("INFOPILOT_RUNTIME_LOG_DIR")
    if runtime_dir:
        log_dir = Path(runtime_dir).expanduser()
    else:
        log_dir = RUNTIME_LOG_DIR
    try:
        log_dir.mkdir(parents=True, exist_ok=True)
        log_path = log_dir / f"{date.today().isoformat()}.log"
        if not any(isinstance(h, logging.FileHandler) and getattr(h, "baseFilename", "") == str(log_path) for h in logger.handlers):
            file_handler = logging.FileHandler(log_path, encoding="utf-8")
            file_handler.setLevel(level)
            file_handler.setFormatter(formatter)
            logger.addHandler(file_handler)
    except Exception:
        # Never fail app execution due to logging config.
        pass

    logger.propagate = False
    return logger
