"""Audit log helpers for meeting pipeline runs."""
from __future__ import annotations

import json
import os
from dataclasses import dataclass
from datetime import date
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Dict, Optional

from core.utils import get_logger
from core.config.paths import AUDIT_LOG_DIR

LOGGER = get_logger("meeting.audit")


@dataclass
class AuditConfig:
    enabled: bool
    log_path: Optional[Path] = None

    @classmethod
    def from_env(cls) -> "AuditConfig":
        raw = (os.getenv("MEETING_AUDIT_LOG") or "").strip()
        if not raw:
            return cls(enabled=False)
        if raw.lower() in {"1", "true", "yes", "on"}:
            log_path = AUDIT_LOG_DIR / f"{date.today().isoformat()}.jsonl"
        else:
            log_path = Path(raw).expanduser()
        log_path.parent.mkdir(parents=True, exist_ok=True)
        return cls(enabled=True, log_path=log_path)


class MeetingAuditLogger:
    """JSONL audit logger recording meeting pipeline events."""

    def __init__(self, config: AuditConfig) -> None:
        self._config = config

    @classmethod
    def from_env(cls) -> "MeetingAuditLogger":
        return cls(AuditConfig.from_env())

    def is_enabled(self) -> bool:
        return self._config.enabled and self._config.log_path is not None

    def record(self, payload: Dict[str, Any]) -> None:
        if not self.is_enabled():
            return
        data = dict(payload)
        data.setdefault("schema_version", 1)
        data.setdefault("event_type", "meeting_pipeline.event")
        data.setdefault("recorded_at", datetime.now(timezone.utc).isoformat())
        try:
            with self._config.log_path.open("a", encoding="utf-8") as handle:
                handle.write(json.dumps(data, ensure_ascii=False) + "\n")
        except Exception as exc:  # pragma: no cover - defensive logging
            LOGGER.warning("failed to write audit log: %s", exc)


__all__ = ["MeetingAuditLogger"]
