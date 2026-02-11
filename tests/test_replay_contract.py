from __future__ import annotations

import json
import sys
from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(PROJECT_ROOT / "src"))

from collector.models import DEFAULT_SCHEMA_VERSION
from collector.normalize import normalize_event


def test_fixture_normalizes_required_fields() -> None:
    fixture = PROJECT_ROOT / "tests" / "fixtures" / "sample_events_os_short.jsonl"
    first_line = fixture.read_text(encoding="utf-8").splitlines()[0]
    raw = json.loads(first_line)
    envelope = normalize_event(raw, validation_level="lenient")
    assert envelope.schema_version == DEFAULT_SCHEMA_VERSION
    assert envelope.event_id
    assert envelope.ts
    assert envelope.source
    assert envelope.app
    assert envelope.event_type
    assert envelope.resource.type
    assert envelope.resource.id
    assert isinstance(envelope.payload, dict)


def test_invalid_schema_version_defaults() -> None:
    raw = {
        "schema_version": "invalid",
        "source": "os",
        "app": "OS",
        "event_type": "os.app_focus_block",
        "resource": {"type": "window", "id": "win-1"},
        "payload": {"duration_sec": 1},
    }
    envelope = normalize_event(raw, validation_level="lenient")
    assert envelope.schema_version == DEFAULT_SCHEMA_VERSION
