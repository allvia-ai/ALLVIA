from __future__ import annotations

import json
from dataclasses import dataclass
from datetime import datetime, timedelta, timezone
from typing import Optional

from .config import RetentionConfig
from .store import SQLiteStore
from .utils.time import utc_now


@dataclass
class RetentionResult:
    deleted_events: int = 0
    deleted_sessions: int = 0
    deleted_routines: int = 0
    deleted_handoff: int = 0
    expired_handoff: int = 0
    db_size_before: int = 0
    db_size_after: int = 0
    vacuumed: bool = False


def run_retention(
    store: SQLiteStore,
    policy: RetentionConfig,
    *,
    now: Optional[datetime] = None,
    force_vacuum: bool = False,
) -> RetentionResult:
    now = now or utc_now()
    result = RetentionResult(db_size_before=store.get_db_size())

    if policy.raw_events_days > 0:
        cutoff = _format_ts(now - timedelta(days=policy.raw_events_days))
        result.deleted_events = store.delete_old_events(
            cutoff, batch_size=policy.batch_size
        )

    if policy.sessions_days > 0:
        cutoff = _format_ts(now - timedelta(days=policy.sessions_days))
        result.deleted_sessions = store.delete_old_sessions(
            cutoff, batch_size=policy.batch_size
        )

    if policy.routine_candidates_days > 0:
        cutoff = _format_ts(now - timedelta(days=policy.routine_candidates_days))
        result.deleted_routines = store.delete_old_routines(
            cutoff, batch_size=policy.batch_size
        )

    if policy.handoff_queue_days > 0:
        cutoff = _format_ts(now - timedelta(days=policy.handoff_queue_days))
        result.expired_handoff = store.expire_pending_handoff(cutoff)
        result.deleted_handoff = store.delete_old_handoff(
            cutoff, batch_size=policy.batch_size
        )

    store.checkpoint_wal()
    db_size_after = store.get_db_size()
    result.db_size_after = db_size_after

    if force_vacuum or _should_vacuum(policy, result.db_size_after):
        store.vacuum()
        result.vacuumed = True
        result.db_size_after = store.get_db_size()

    return result


def retention_result_json(result: RetentionResult) -> str:
    payload = {
        "event": "retention",
        "deleted_events": result.deleted_events,
        "deleted_sessions": result.deleted_sessions,
        "deleted_routines": result.deleted_routines,
        "deleted_handoff": result.deleted_handoff,
        "expired_handoff": result.expired_handoff,
        "db_size_before": result.db_size_before,
        "db_size_after": result.db_size_after,
        "vacuumed": result.vacuumed,
    }
    return json.dumps(payload, separators=(",", ":"))


def _format_ts(value: datetime) -> str:
    if value.tzinfo is None:
        value = value.replace(tzinfo=timezone.utc)
    return value.isoformat().replace("+00:00", "Z")


def _should_vacuum(policy: RetentionConfig, db_size_bytes: int) -> bool:
    if policy.max_db_mb <= 0:
        return False
    return db_size_bytes >= policy.max_db_mb * 1024 * 1024
