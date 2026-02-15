from __future__ import annotations

import json
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Dict, Optional


class AuditJSONLLogger:
    """Append-only JSONL audit logger (best-effort, no exceptions by default)."""

    def __init__(self, path: Path, *, enabled: bool = True, schema_version: int = 1) -> None:
        self._path = path
        self._enabled = enabled
        self._schema_version = schema_version
        if enabled:
            self._path.parent.mkdir(parents=True, exist_ok=True)

    @property
    def path(self) -> Path:
        return self._path

    def is_enabled(self) -> bool:
        return self._enabled

    def record(self, payload: Dict[str, Any], *, event_type: Optional[str] = None) -> None:
        if not self._enabled:
            return
        data: Dict[str, Any] = dict(payload)
        data.setdefault("schema_version", self._schema_version)
        data.setdefault("recorded_at", datetime.now(timezone.utc).isoformat())
        if event_type:
            data.setdefault("event_type", event_type)

        with self._path.open("a", encoding="utf-8") as handle:
            handle.write(json.dumps(data, ensure_ascii=False) + "\n")

