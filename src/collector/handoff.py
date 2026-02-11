from __future__ import annotations

import json
import re
import uuid
from collections import Counter
from dataclasses import dataclass
from datetime import datetime, timedelta, timezone
from typing import Any, Dict, Iterable, List, Optional, Tuple

from .privacy import PrivacyRules, load_privacy_rules
from .store import SQLiteStore
from .utils.masking import mask_patterns, truncate
from .utils.time import utc_now

DEFAULT_MAX_SIZE_BYTES = 50 * 1024
DEFAULT_RECENT_SESSIONS = 3
DEFAULT_RECENT_ROUTINES = 10
DEFAULT_MAX_RESOURCES = 10
DEFAULT_MAX_EVIDENCE = 5
DEFAULT_REDACTION_SCAN_LIMIT = 200

EMAIL_RE = re.compile(r"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}")
PATH_RE = re.compile(r"([A-Za-z]:\\\\|/Users/|/home/|\\.xlsx|\\.docx|\\.pptx)")
LONG_DIGITS_RE = re.compile(r"\b\d{12,}\b")
HEX64_RE = re.compile(r"^[a-f0-9]{64}$", re.IGNORECASE)


@dataclass
class HandoffPayload:
    payload: Dict[str, Any]
    size_bytes: int


def build_handoff_with_size_guard(
    store: SQLiteStore,
    privacy_rules_path: str,
    *,
    max_size_bytes: int = DEFAULT_MAX_SIZE_BYTES,
    recent_sessions: int = DEFAULT_RECENT_SESSIONS,
    recent_routines: int = DEFAULT_RECENT_ROUTINES,
    max_resources: int = DEFAULT_MAX_RESOURCES,
    max_evidence: int = DEFAULT_MAX_EVIDENCE,
    redaction_scan_limit: int = DEFAULT_REDACTION_SCAN_LIMIT,
) -> HandoffPayload:
    package_id = str(uuid.uuid4())
    created_at = _format_ts(utc_now())

    profiles = [
        (recent_sessions, recent_routines, max_resources),
        (min(2, recent_sessions), recent_routines, max_resources),
        (1, min(5, recent_routines), min(5, max_resources)),
        (1, min(3, recent_routines), min(3, max_resources)),
        (1, 1, 1),
    ]

    rules = load_privacy_rules(privacy_rules_path)
    last_payload: Optional[Dict[str, Any]] = None
    last_size = 0

    for sessions_limit, routines_limit, resources_limit in profiles:
        payload = _build_handoff_payload(
            store,
            rules,
            package_id,
            created_at,
            sessions_limit,
            routines_limit,
            resources_limit,
            max_evidence,
            redaction_scan_limit,
        )
        payload = _scrub_payload(payload)
        size_bytes = _payload_size(payload)
        last_payload = payload
        last_size = size_bytes
        if size_bytes <= max_size_bytes:
            return HandoffPayload(payload=payload, size_bytes=size_bytes)

    return HandoffPayload(payload=last_payload or {}, size_bytes=last_size)


def _build_handoff_payload(
    store: SQLiteStore,
    rules: PrivacyRules,
    package_id: str,
    created_at: str,
    sessions_limit: int,
    routines_limit: int,
    resources_limit: int,
    max_evidence: int,
    redaction_scan_limit: int,
) -> Dict[str, Any]:
    device_context = _device_context(store, rules)
    recent_sessions = _recent_sessions(store, sessions_limit, resources_limit)
    routine_candidates = _routine_candidates(store, routines_limit, max_evidence)
    signals = _signals(store, device_context.get("last_event_ts"))
    privacy_state = _privacy_state(store, rules, redaction_scan_limit)

    return {
        "package_id": package_id,
        "created_at": created_at,
        "version": "1.0",
        "device_context": device_context,
        "recent_sessions": recent_sessions,
        "routine_candidates": routine_candidates,
        "signals": signals,
        "privacy_state": privacy_state,
    }


def _device_context(store: SQLiteStore, rules: PrivacyRules) -> Dict[str, Any]:
    latest = store.fetch_latest_event()
    if not latest:
        return {"active_app": None, "active_window_hint": None, "last_event_ts": None}

    ts, event_type, _priority, app, payload_json = latest
    payload = _safe_json(payload_json)
    window_title = payload.get("window_title")
    window_hint = _sanitize_hint(str(window_title), rules) if window_title else None
    return {
        "active_app": app,
        "active_window_hint": window_hint,
        "last_event_ts": ts,
        "last_event_type": event_type,
    }


def _signals(store: SQLiteStore, last_event_ts: Optional[str]) -> Dict[str, Any]:
    now = utc_now()
    since = _format_ts(now - timedelta(minutes=5))
    p0_recent = store.has_recent_p0(since)
    idle_state = None
    if last_event_ts:
        latest = store.fetch_latest_event()
        if latest:
            event_type = (latest[1] or "").lower()
            if event_type == "os.idle_start":
                idle_state = True
            elif event_type == "os.idle_end":
                idle_state = False
    return {"p0_recent": p0_recent, "idle_state": idle_state}


def _privacy_state(
    store: SQLiteStore, rules: PrivacyRules, redaction_scan_limit: int
) -> Dict[str, Any]:
    redaction_summary = _redaction_summary(store.fetch_recent_privacy(redaction_scan_limit))
    return {
        "content_collection": False,
        "denylist_active": bool(rules.denylist_apps),
        "redaction_summary": redaction_summary,
    }


def _recent_sessions(
    store: SQLiteStore, limit: int, max_resources: int
) -> List[Dict[str, Any]]:
    rows = store.fetch_recent_sessions(limit)
    sessions: List[Dict[str, Any]] = []
    for row in rows:
        session_id, start_ts, end_ts, duration_sec, summary_json = row
        summary = _safe_json(summary_json)
        resources = summary.get("resources", [])
        if isinstance(resources, list):
            resources = resources[:max_resources]
        else:
            resources = []
        sessions.append(
            {
                "session_id": session_id,
                "start_ts": start_ts,
                "end_ts": end_ts,
                "duration_sec": duration_sec,
                "apps_timeline": summary.get("apps_timeline", []),
                "key_events": summary.get("key_events", []),
                "resources": resources,
                "counts": summary.get("counts", {}),
            }
        )
    return sessions


def _routine_candidates(
    store: SQLiteStore, limit: int, max_evidence: int
) -> List[Dict[str, Any]]:
    rows = store.fetch_routine_candidates(limit)
    candidates: List[Dict[str, Any]] = []
    for row in rows:
        pattern_id, pattern_json, support, confidence, last_seen_ts, evidence_json = row
        pattern = _safe_json(pattern_json)
        evidence = _safe_list(evidence_json)
        if max_evidence > 0:
            evidence = evidence[:max_evidence]
        candidates.append(
            {
                "pattern_id": pattern_id,
                "pattern": pattern,
                "support": support,
                "confidence": confidence,
                "last_seen_ts": last_seen_ts,
                "evidence_session_ids": evidence,
            }
        )
    return candidates


def _redaction_summary(rows: Iterable[tuple]) -> Dict[str, Any]:
    counter: Counter[str] = Counter()
    total = 0
    for (privacy_json,) in rows:
        data = _safe_json(privacy_json)
        redaction = data.get("redaction")
        if isinstance(redaction, list):
            for item in redaction:
                if item:
                    counter[str(item)] += 1
                    total += 1
    top = dict(counter.most_common(10))
    return {"total": total, "items": top}


def _sanitize_hint(value: str, rules: PrivacyRules) -> str:
    masked = mask_patterns(value, rules.redaction_patterns)
    max_len = rules.length_limits.get("window_title", 64)
    masked = truncate(masked, max_len)
    return _scrub_string(masked)


def _scrub_payload(value: Any) -> Any:
    if isinstance(value, dict):
        return {key: _scrub_payload(val) for key, val in value.items()}
    if isinstance(value, list):
        return [_scrub_payload(item) for item in value]
    if isinstance(value, str):
        return _scrub_string(value)
    return value


def _scrub_string(value: str) -> str:
    if HEX64_RE.match(value):
        return value
    if EMAIL_RE.search(value) or PATH_RE.search(value) or LONG_DIGITS_RE.search(value):
        return "[REDACTED]"
    return value


def _payload_size(payload: Dict[str, Any]) -> int:
    return len(json.dumps(payload, separators=(",", ":")).encode("utf-8"))


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


def _safe_list(value: Any) -> List[Any]:
    if not value:
        return []
    try:
        if isinstance(value, str):
            parsed = json.loads(value)
            return parsed if isinstance(parsed, list) else []
        if isinstance(value, list):
            return value
    except (TypeError, ValueError, json.JSONDecodeError):
        return []
    return []
