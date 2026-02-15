from __future__ import annotations

from pathlib import Path

from core.infra import AuditLogger


def test_audit_logger_writes_file(tmp_path: Path) -> None:
    logger = AuditLogger(tmp_path / "audit")
    payload = {"event": "test", "value": 123}
    log_path = logger.log("test-event", payload)
    assert log_path.exists()
    assert "test" in log_path.read_text(encoding="utf-8")
