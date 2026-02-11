from __future__ import annotations

from collections import Counter
from typing import Any, Dict, Iterable, List

from .models import SessionEvent

KEY_P1_TYPES = {
    "outlook.compose_started",
    "outlook.attachment_added_meta",
    "excel.refresh_pivot",
}

MAX_RESOURCES = 20


def build_session_summary(events: Iterable[SessionEvent]) -> Dict[str, Any]:
    events_list = list(events)
    apps_timeline = _apps_timeline(events_list)
    key_events = _key_events(events_list)
    resources = _resources(events_list)
    counts = _counts(events_list)
    return {
        "apps_timeline": apps_timeline,
        "key_events": key_events,
        "resources": resources,
        "counts": counts,
    }


def _apps_timeline(events: Iterable[SessionEvent]) -> List[Dict[str, Any]]:
    totals: Dict[str, int] = {}
    for event in events:
        if (event.event_type or "").lower() != "os.app_focus_block":
            continue
        duration = _safe_int(event.payload.get("duration_sec"))
        if duration <= 0:
            continue
        app = event.app or "unknown"
        totals[app] = totals.get(app, 0) + duration
    timeline = [{"app": app, "sec": sec} for app, sec in totals.items()]
    timeline.sort(key=lambda item: item["sec"], reverse=True)
    return timeline


def _key_events(events: Iterable[SessionEvent]) -> List[str]:
    seen = set()
    ordered: List[str] = []
    for event in events:
        event_type = (event.event_type or "").lower()
        if not event_type:
            continue
        include = (event.priority or "").upper() == "P0" or event_type in KEY_P1_TYPES
        if include and event_type not in seen:
            seen.add(event_type)
            ordered.append(event_type)
    return ordered


def _resources(events: Iterable[SessionEvent]) -> List[Dict[str, str]]:
    seen = set()
    output: List[Dict[str, str]] = []
    for event in events:
        key = (event.resource_type, event.resource_id)
        if key in seen:
            continue
        seen.add(key)
        output.append({"type": event.resource_type, "id": event.resource_id})
        if len(output) >= MAX_RESOURCES:
            break
    return output


def _counts(events: List[SessionEvent]) -> Dict[str, int]:
    counter = Counter((event.priority or "").upper() for event in events if event.priority)
    return {
        "total": len(events),
        "p0": counter.get("P0", 0),
        "p1": counter.get("P1", 0),
        "p2": counter.get("P2", 0),
    }


def _safe_int(value: Any) -> int:
    try:
        return int(value)
    except (TypeError, ValueError):
        return 0
