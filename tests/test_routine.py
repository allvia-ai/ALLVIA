from __future__ import annotations

import json
import sys
from datetime import datetime, timedelta, timezone
from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(PROJECT_ROOT / "src"))

from collector.routine import RoutineSession, build_routine_candidates


def test_build_routine_candidates_support_and_evidence() -> None:
    base = datetime(2026, 1, 21, 0, 0, 0, tzinfo=timezone.utc)
    sessions = [
        RoutineSession(
            session_id="s1",
            start_ts=base,
            end_ts=base + timedelta(minutes=5),
            key_events=["excel.export_pdf", "outlook.send_clicked"],
        ),
        RoutineSession(
            session_id="s2",
            start_ts=base + timedelta(days=1),
            end_ts=base + timedelta(days=1, minutes=5),
            key_events=["excel.export_pdf", "outlook.send_clicked"],
        ),
        RoutineSession(
            session_id="s3",
            start_ts=base + timedelta(days=2),
            end_ts=base + timedelta(days=2, minutes=5),
            key_events=["excel.export_pdf", "outlook.compose_started"],
        ),
    ]

    candidates = build_routine_candidates(
        sessions,
        n_min=2,
        n_max=2,
        min_support=2,
        max_patterns=10,
        max_evidence=2,
    )

    assert candidates
    candidate = candidates[0]
    pattern = json.loads(candidate.pattern_json)
    assert pattern["events"] == ["excel.export_pdf", "outlook.send_clicked"]
    assert candidate.support == 2
    assert len(candidate.evidence_session_ids) <= 2
