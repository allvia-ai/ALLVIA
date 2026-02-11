from __future__ import annotations

import json
import re
import sys
from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(PROJECT_ROOT / "src"))

from collector.normalize import normalize_event
from collector.privacy import PrivacyGuard, PrivacyRules


def _guard() -> PrivacyGuard:
    rules = PrivacyRules(
        mask_keys={"window_title"},
        hash_keys={"path", "file_path"},
        length_limits={"window_title": 16},
        url_policy={"allow_full_url": False, "keep_domain_only": True},
        redaction_patterns=[
            re.compile(r"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}"),
        ],
        drop_payload_keys={"body"},
        allowlist_apps=set(),
        denylist_apps={"blocked"},
        denylist_action="drop",
    )
    return PrivacyGuard(rules, hash_salt="salt")


def test_privacy_masks_and_hashes() -> None:
    raw = {
        "schema_version": "1.0",
        "ts": "2026-01-21T00:00:00Z",
        "source": "os",
        "app": "OS",
        "event_type": "os.app_focus_block",
        "resource": {"type": "window", "id": "win-1"},
        "window_id": "win-1",
        "payload": {
            "window_title": "email alice@example.com",
            "path": "/tmp/secret",
        },
        "privacy": {"pii_level": "unknown", "redaction": []},
    }
    envelope = normalize_event(raw, validation_level="lenient")
    out = _guard().apply(envelope)
    assert out is not None
    assert "[REDACTED]" in out.payload["window_title"]
    assert len(out.payload["window_title"]) <= 16
    assert out.resource.id != "win-1"
    assert len(out.resource.id) == 64
    assert out.window_id is not None and len(out.window_id) == 64
    assert out.payload["path"] != "/tmp/secret"
    assert len(out.payload["path"]) == 64


def test_privacy_summarizes_recipients() -> None:
    raw = {
        "schema_version": "1.0",
        "ts": "2026-01-21T00:01:00Z",
        "source": "outlook_addin",
        "app": "OUTLOOK",
        "event_type": "outlook.send_clicked",
        "resource": {"type": "email", "id": "draft-1"},
        "payload": {
            "recipients": ["alice@example.com", "bob@test.com"],
            "to": ["ceo@corp.com"],
            "cc": ["team@corp.com"],
            "attachments_count": 1,
        },
        "privacy": {"pii_level": "unknown", "redaction": []},
    }
    envelope = normalize_event(raw, validation_level="lenient")
    out = _guard().apply(envelope)
    assert out is not None
    payload_text = json.dumps(out.payload)
    assert "@" not in payload_text
    assert out.payload["recipients"]["count"] == 2


def test_privacy_drops_denylisted_app() -> None:
    raw = {
        "schema_version": "1.0",
        "ts": "2026-01-21T00:02:00Z",
        "source": "os",
        "app": "BLOCKED",
        "event_type": "os.app_focus_block",
        "resource": {"type": "window", "id": "win-9"},
        "payload": {"window_title": "blocked"},
        "privacy": {"pii_level": "unknown", "redaction": []},
    }
    envelope = normalize_event(raw, validation_level="lenient")
    out = _guard().apply(envelope)
    assert out is None
