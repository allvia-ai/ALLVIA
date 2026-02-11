from __future__ import annotations

import hashlib
import json
from collections import Counter
from dataclasses import dataclass, field
from datetime import datetime, timezone
from typing import Any, Dict, Iterable, List, Optional, Sequence, Tuple

from .utils.time import parse_ts, utc_now


@dataclass
class RoutineSession:
    session_id: str
    start_ts: datetime
    end_ts: datetime
    key_events: List[str]


@dataclass
class RoutineCandidate:
    pattern_id: str
    pattern_json: str
    support: int
    confidence: float
    last_seen_ts: str
    evidence_session_ids: List[str] = field(default_factory=list)


@dataclass
class _PatternStats:
    support: int = 0
    session_ids: List[str] = field(default_factory=list)
    session_id_set: set[str] = field(default_factory=set)
    last_seen: Optional[datetime] = None
    weekday_counts: Counter[int] = field(default_factory=Counter)


def rows_to_sessions(rows: Iterable[tuple]) -> List[RoutineSession]:
    sessions: List[RoutineSession] = []
    for row in rows:
        session_id, start_ts, end_ts, summary_json = row
        start_parsed = parse_ts(start_ts)
        end_parsed = parse_ts(end_ts)
        if start_parsed is None or end_parsed is None:
            continue
        summary = _safe_json(summary_json)
        key_events = summary.get("key_events", [])
        if not isinstance(key_events, list):
            continue
        events = [str(item).lower() for item in key_events if item]
        sessions.append(
            RoutineSession(
                session_id=str(session_id),
                start_ts=start_parsed,
                end_ts=end_parsed,
                key_events=events,
            )
        )
    sessions.sort(key=lambda item: item.start_ts)
    return sessions


def build_routine_candidates(
    sessions: Sequence[RoutineSession],
    *,
    n_min: int = 2,
    n_max: int = 5,
    min_support: int = 2,
    max_patterns: int = 100,
    max_evidence: int = 10,
) -> List[RoutineCandidate]:
    if max_patterns <= 0:
        return []

    stats: Dict[Tuple[str, ...], _PatternStats] = {}
    for session in sessions:
        if len(session.key_events) < n_min:
            continue
        patterns = _unique_ngrams(session.key_events, n_min, n_max)
        if not patterns:
            continue
        weekday = session.start_ts.weekday()
        for pattern in patterns:
            entry = stats.setdefault(pattern, _PatternStats())
            if session.session_id in entry.session_id_set:
                continue
            entry.session_id_set.add(session.session_id)
            entry.session_ids.append(session.session_id)
            entry.support += 1
            entry.weekday_counts[weekday] += 1
            if entry.last_seen is None or session.end_ts > entry.last_seen:
                entry.last_seen = session.end_ts

    now = utc_now()
    candidates: List[RoutineCandidate] = []
    for pattern, entry in stats.items():
        if entry.support < min_support:
            continue
        last_seen = entry.last_seen or now
        confidence = _confidence(entry.support, entry.weekday_counts, last_seen, now)
        pattern_json = json.dumps(
            {"type": "ngram", "events": list(pattern), "n": len(pattern)},
            separators=(",", ":"),
        )
        pattern_id = _hash_pattern(pattern_json)
        evidence = entry.session_ids[-max_evidence:] if max_evidence > 0 else []
        candidates.append(
            RoutineCandidate(
                pattern_id=pattern_id,
                pattern_json=pattern_json,
                support=entry.support,
                confidence=confidence,
                last_seen_ts=_format_ts(last_seen),
                evidence_session_ids=evidence,
            )
        )

    candidates.sort(key=lambda item: (item.support, item.confidence), reverse=True)
    return candidates[:max_patterns]


def _unique_ngrams(events: Sequence[str], n_min: int, n_max: int) -> set[Tuple[str, ...]]:
    if n_min <= 0 or n_max < n_min:
        return set()
    limit = min(n_max, len(events))
    output: set[Tuple[str, ...]] = set()
    for n in range(n_min, limit + 1):
        for idx in range(len(events) - n + 1):
            output.add(tuple(events[idx : idx + n]))
    return output


def _confidence(
    support: int,
    weekday_counts: Counter[int],
    last_seen: datetime,
    now: datetime,
) -> float:
    recency_bonus = 0.0
    days_ago = (now - last_seen).days
    if days_ago <= 1:
        recency_bonus = 0.3
    elif days_ago <= 7:
        recency_bonus = 0.1

    periodicity_bonus = 0.0
    if any(count >= 2 for count in weekday_counts.values()):
        periodicity_bonus = 0.1

    return support * (1 + recency_bonus) * (1 + periodicity_bonus)


def _hash_pattern(value: str) -> str:
    return hashlib.sha256(value.encode("utf-8")).hexdigest()


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
