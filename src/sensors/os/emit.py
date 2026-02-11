from __future__ import annotations

import datetime as dt
import json
import logging
import time
import urllib.error
import urllib.request
from dataclasses import dataclass
from typing import Any, Dict, Iterable, List, Optional
from uuid import uuid4

logger = logging.getLogger(__name__)


@dataclass
class EmitConfig:
    ingest_url: str = "http://127.0.0.1:8080/events"
    timeout_sec: float = 2.0
    retries: int = 3
    backoff_sec: float = 0.5


def utc_now() -> str:
    return dt.datetime.utcnow().isoformat() + "Z"


def build_event(
    *,
    source: str,
    app: str,
    event_type: str,
    resource_type: str,
    resource_id: str,
    payload: Optional[Dict[str, Any]] = None,
    priority: str = "P1",
    window_id: Optional[str] = None,
    pid: Optional[int] = None,
) -> Dict[str, Any]:
    return {
        "schema_version": "1.0",
        "event_id": str(uuid4()),
        "ts": utc_now(),
        "source": source,
        "app": app,
        "event_type": event_type,
        "priority": priority,
        "resource": {
            "type": resource_type,
            "id": resource_id,
        },
        "payload": payload or {},
        "privacy": {"pii_level": "unknown", "redaction": []},
        "window_id": window_id,
        "pid": pid,
    }


class HttpEmitter:
    def __init__(self, config: EmitConfig) -> None:
        self._config = config

    def send_event(self, event: Dict[str, Any]) -> bool:
        return self.send_events([event])

    def send_events(self, events: Iterable[Dict[str, Any]]) -> bool:
        payload = list(events)
        data = json.dumps(payload).encode("utf-8")
        request = urllib.request.Request(
            self._config.ingest_url,
            data=data,
            headers={"Content-Type": "application/json"},
            method="POST",
        )

        for attempt in range(self._config.retries):
            try:
                with urllib.request.urlopen(
                    request, timeout=self._config.timeout_sec
                ) as response:
                    if 200 <= response.status < 300:
                        return True
                    logger.warning("ingest responded with %s", response.status)
            except urllib.error.URLError as exc:
                logger.warning("ingest send failed: %s", exc)

            delay = self._config.backoff_sec * (2**attempt)
            time.sleep(delay)

        return False
