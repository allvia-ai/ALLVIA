"""Audit logging utilities."""
from __future__ import annotations

import json
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Dict

from core.utils import get_logger

LOGGER = get_logger("infra.audit")


class AuditLogger:
    def __init__(self, log_dir: Path) -> None:
        self.log_dir = log_dir
        self.log_dir.mkdir(parents=True, exist_ok=True)

    def log(self, event: str, payload: Dict[str, Any]) -> Path:
        timestamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
        log_path = self.log_dir / f"{timestamp}_{event}.json"
        log_path.write_text(json.dumps(payload, ensure_ascii=False, indent=2), encoding="utf-8")
        LOGGER.info("audit event logged: %s", log_path)
        return log_path
