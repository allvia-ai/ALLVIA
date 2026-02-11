from __future__ import annotations

import json
import sys
from datetime import datetime, timedelta, timezone
from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(PROJECT_ROOT / "src"))

from collector.features import build_session_summary
from collector.models import SessionEvent
from collector.sessionizer import build_session_records, sessionize


def _event(ts: datetime, event_type: str, priority: str = "P1", app: str = "OS") -> SessionEvent:
    return SessionEvent(
        ts=ts,
        event_type=event_type,
        priority=priority,
        app=app,
        resource_type="window",
        resource_id="r1",
        payload={"duration_sec": 5},
    )


def test_sessionize_idle_and_gap() -> None:
    base = datetime(2026, 1, 21, 0, 0, 0, tzinfo=timezone.utc)
    events = [
        _event(base, "os.app_focus_block"),
        _event(base + timedelta(seconds=5), "os.idle_start"),
        _event(base + timedelta(minutes=20), "os.app_focus_block"),
    ]
    sessions = sessionize(events, gap_seconds=600)
    assert len(sessions) == 2
    assert len(sessions[0]) == 1
    assert len(sessions[1]) == 1


def test_sessionize_p0_closes_session() -> None:
    base = datetime(2026, 1, 21, 1, 0, 0, tzinfo=timezone.utc)
    events = [
        _event(base, "os.app_focus_block"),
        _event(base + timedelta(seconds=5), "outlook.send_clicked", priority="P0", app="OUTLOOK"),
        _event(base + timedelta(seconds=8), "os.app_focus_block"),
    ]
    sessions = sessionize(events, gap_seconds=600)
    assert len(sessions) == 2
    assert len(sessions[0]) == 2
    assert len(sessions[1]) == 1


def test_session_summary_fields() -> None:
    base = datetime(2026, 1, 21, 2, 0, 0, tzinfo=timezone.utc)
    events = [
        _event(base, "os.app_focus_block", app="EXCEL"),
        _event(base + timedelta(seconds=10), "outlook.compose_started", app="OUTLOOK"),
    ]
    summary = build_session_summary(events)
    assert "apps_timeline" in summary
    assert "key_events" in summary
    assert "resources" in summary
    assert "counts" in summary
    assert summary["counts"]["total"] == 2

    record = build_session_records([events])[0]
    parsed = json.loads(record.summary_json)
    assert parsed["counts"]["total"] == 2
