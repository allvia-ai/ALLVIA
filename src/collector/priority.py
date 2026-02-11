from __future__ import annotations

import uuid
from dataclasses import dataclass, field
from typing import Dict, List, Optional, Tuple

from .models import EventEnvelope, PrivacyMetadata, ResourceRef, VALID_PRIORITIES

try:
    from .observability import Observability
except ImportError:  # pragma: no cover - optional for test import order
    Observability = None  # type: ignore
from .utils.time import parse_ts, utc_now

P0_EVENT_TYPES = {
    "outlook.send_clicked",
    "excel.export_pdf",
    "excel.export_csv",
    "excel.save_as",
    "os.file_saved",
    "excel.refresh_pivot",
    "upload_done",
    "share_link_created",
}

P1_EVENT_TYPES = {
    "os.app_focus_block",
    "os.file_opened",
    "excel.workbook_opened",
    "outlook.compose_started",
    "outlook.attachment_added_meta",
}

P2_EVENT_TYPES = {
    "os.foreground_changed",
    "os.window_title_changed",
    "os.clipboard_meta",
}

DEBOUNCE_EVENT_TYPES = {
    "os.foreground_changed",
    "os.window_title_changed",
}


@dataclass
class FocusState:
    envelope: EventEnvelope
    ts: Optional[float]


@dataclass
class PriorityProcessor:
    debounce_seconds: float = 2.0
    focus_event_types: List[str] = field(default_factory=lambda: ["os.foreground_changed"])
    focus_block_event_type: str = "os.app_focus_block"
    drop_p2_when_queue_over: float = 0.8
    metrics: Optional["Observability"] = None

    _last_event_ts: Dict[Tuple[str, str, str], float] = field(default_factory=dict)
    _focus_state: Optional[FocusState] = None

    def process(self, envelope: EventEnvelope, queue_ratio: float) -> List[EventEnvelope]:
        event_type = (envelope.event_type or "").lower()
        envelope.priority = _classify_priority(event_type, envelope.priority)

        if envelope.priority == "P2" and queue_ratio >= self.drop_p2_when_queue_over:
            if self.metrics:
                self.metrics.record_drop("queue_overflow")
            return []

        if event_type in self._focus_event_types_set():
            return self._handle_focus_event(envelope)

        if event_type in DEBOUNCE_EVENT_TYPES:
            if self._should_debounce(envelope, event_type):
                if self.metrics:
                    self.metrics.record_drop("debounce")
                return []

        return [envelope]

    def flush(self) -> List[EventEnvelope]:
        if not self._focus_state:
            return []
        now = utc_now().timestamp()
        return self._emit_focus_block(now)

    def _focus_event_types_set(self) -> set[str]:
        return {item.lower() for item in self.focus_event_types}

    def _should_debounce(self, envelope: EventEnvelope, event_type: str) -> bool:
        ts = _to_epoch(envelope.ts)
        if ts is None:
            return False
        key = (event_type, envelope.app, envelope.resource.id)
        last_ts = self._last_event_ts.get(key)
        self._last_event_ts[key] = ts
        if last_ts is None:
            return False
        return (ts - last_ts) < self.debounce_seconds

    def _handle_focus_event(self, envelope: EventEnvelope) -> List[EventEnvelope]:
        ts = _to_epoch(envelope.ts)
        emitted: List[EventEnvelope] = []
        if self._focus_state and ts is not None:
            emitted = self._emit_focus_block(ts)
        self._focus_state = FocusState(envelope=envelope, ts=ts)
        return emitted

    def _emit_focus_block(self, ts: float) -> List[EventEnvelope]:
        if not self._focus_state:
            return []
        prev = self._focus_state
        if prev.ts is None:
            return []
        duration = max(0.0, ts - prev.ts)
        if duration < self.debounce_seconds:
            return []

        payload = dict(prev.envelope.payload)
        payload["duration_sec"] = int(duration)

        block_event = EventEnvelope(
            schema_version=prev.envelope.schema_version,
            event_id=str(uuid.uuid4()),
            ts=prev.envelope.ts,
            source=prev.envelope.source,
            app=prev.envelope.app,
            event_type=self.focus_block_event_type,
            priority=_classify_priority(self.focus_block_event_type, "P1"),
            resource=ResourceRef(
                type=prev.envelope.resource.type,
                id=prev.envelope.resource.id,
            ),
            payload=payload,
            privacy=PrivacyMetadata(
                pii_level=prev.envelope.privacy.pii_level,
                redaction=list(prev.envelope.privacy.redaction),
            ),
            pid=prev.envelope.pid,
            window_id=prev.envelope.window_id,
            raw=prev.envelope.raw,
        )

        return [block_event]


def _classify_priority(event_type: str, current: str) -> str:
    if event_type in P0_EVENT_TYPES:
        return "P0"
    if event_type in P1_EVENT_TYPES:
        return "P1"
    if event_type in P2_EVENT_TYPES:
        return "P2"
    if current in VALID_PRIORITIES:
        return current
    return "P1"


def _to_epoch(ts_value: str) -> Optional[float]:
    parsed = parse_ts(ts_value)
    if parsed is None:
        return None
    return parsed.timestamp()
