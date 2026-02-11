from __future__ import annotations

import json
import uuid
from dataclasses import dataclass
from datetime import datetime, timezone
from typing import Any, Dict, Iterable, List, Optional, Sequence

from .features import build_session_summary
from .models import SessionEvent
from .utils.time import parse_ts


IDLE_START_EVENT = "os.idle_start"


@dataclass
class SessionRecord:
    session_id: str
    start_ts: str
    end_ts: str
    duration_sec: int
    summary_json: str


def rows_to_events(rows: Iterable[tuple]) -> List[SessionEvent]:
    events: List[SessionEvent] = []
    for row in rows:
        ts_raw, event_type, priority, app, resource_type, resource_id, payload_json = row
        parsed = parse_ts(ts_raw)
        if parsed is None:
            continue
        payload = _safe_json(payload_json)
        events.append(
            SessionEvent(
                ts=parsed,
                event_type=str(event_type or ""),
                priority=str(priority or ""),
                app=str(app or ""),
                resource_type=str(resource_type or ""),
                resource_id=str(resource_id or ""),
                payload=payload,
            )
        )
    events.sort(key=lambda item: item.ts)
    return events


def sessionize(
    events: Sequence[SessionEvent],
    gap_seconds: int = 900,
) -> List[List[SessionEvent]]:
    sessions: List[List[SessionEvent]] = []
    current: List[SessionEvent] = []
    last_ts: Optional[datetime] = None

    for event in events:
        if last_ts is not None and gap_seconds > 0:
            gap = (event.ts - last_ts).total_seconds()
            if gap >= gap_seconds:
                _flush_session(current, sessions)
                current = []
                last_ts = None

        if (event.event_type or "").lower() == IDLE_START_EVENT:
            _flush_session(current, sessions)
            current = []
            last_ts = None
            continue

        current.append(event)

        if (event.priority or "").upper() == "P0":
            _flush_session(current, sessions)
            current = []
            last_ts = None
            continue

        last_ts = event.ts

    _flush_session(current, sessions)
    return sessions


def build_session_records(
    sessions: Iterable[Sequence[SessionEvent]],
) -> List[SessionRecord]:
    records: List[SessionRecord] = []
    for events in sessions:
        if not events:
            continue
        start_ts = events[0].ts
        end_ts = events[-1].ts
        duration = max(0, int((end_ts - start_ts).total_seconds()))
        summary = build_session_summary(events)
        records.append(
            SessionRecord(
                session_id=str(uuid.uuid4()),
                start_ts=_format_ts(start_ts),
                end_ts=_format_ts(end_ts),
                duration_sec=duration,
                summary_json=json.dumps(summary, separators=(",", ":")),
            )
        )
    return records


def _flush_session(current: List[SessionEvent], sessions: List[List[SessionEvent]]) -> None:
    if current:
        sessions.append(list(current))


def _format_ts(value: datetime) -> str:
    if value.tzinfo is None:
        value = value.replace(tzinfo=timezone.utc)
    return value.isoformat().replace("+00:00", "Z")


def _safe_json(value: Any) -> Dict[str, Any]:
    if not value:
        return {}
    try:
        if isinstance(value, str):
            return json.loads(value)
        if isinstance(value, dict):
            return value
    except (TypeError, ValueError, json.JSONDecodeError):
        return {}
    return {}
