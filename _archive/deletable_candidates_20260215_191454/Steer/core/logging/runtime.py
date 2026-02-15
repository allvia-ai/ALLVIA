from __future__ import annotations

import logging
import os
from datetime import date
from pathlib import Path
from typing import Optional

from core.config.paths import RUNTIME_LOG_DIR

DEFAULT_FORMAT = "%(asctime)s | %(levelname)s | %(name)s: %(message)s"


def _resolve_level() -> int:
    raw = (os.getenv("INFOPILOT_LOG_LEVEL") or "INFO").strip().upper()
    return getattr(logging, raw, logging.INFO)


def configure_runtime_logging(
    *,
    log_dir: Optional[Path] = None,
    logger_name: str = "infopilot",
    also_stderr: bool = True,
) -> Path:
    """Attach a daily runtime log file handler to the project logger.

    Idempotent: calling multiple times won't duplicate handlers.
    """
    level = _resolve_level()
    target_dir = (log_dir or Path(os.getenv("INFOPILOT_RUNTIME_LOG_DIR") or RUNTIME_LOG_DIR)).expanduser()
    target_dir.mkdir(parents=True, exist_ok=True)

    log_path = target_dir / f"{date.today().isoformat()}.log"
    formatter = logging.Formatter(DEFAULT_FORMAT)

    logger = logging.getLogger(logger_name)
    logger.setLevel(level)

    if also_stderr and not any(isinstance(h, logging.StreamHandler) for h in logger.handlers):
        stream = logging.StreamHandler()
        stream.setLevel(level)
        stream.setFormatter(formatter)
        logger.addHandler(stream)

    if not any(isinstance(h, logging.FileHandler) and getattr(h, "baseFilename", "") == str(log_path) for h in logger.handlers):
        file_handler = logging.FileHandler(log_path, encoding="utf-8")
        file_handler.setLevel(level)
        file_handler.setFormatter(formatter)
        logger.addHandler(file_handler)

    logger.propagate = False
    return log_path

