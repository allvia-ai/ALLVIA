"""JSONL audit logging helpers for desktop agent scripts."""
from __future__ import annotations

import json
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Dict, Optional

REPO_ROOT = Path(__file__).resolve().parents[1]
try:
    from core.config.paths import AUDIT_LOG_DIR  # type: ignore
except Exception:  # pragma: no cover - best-effort fallback
    AUDIT_LOG_DIR = REPO_ROOT / "artifacts" / "logs" / "audit"  # type: ignore[assignment]

DEFAULT_AUDIT_PATH = AUDIT_LOG_DIR / "desktop_agents.jsonl"


def _ensure_path(path: Path) -> Path:
    path.parent.mkdir(parents=True, exist_ok=True)
    return path


def record_event(
    *,
    agent: str,
    event: str,
    status: str,
    details: Optional[Dict[str, Any]] = None,
    log_path: Optional[Path] = None,
) -> None:
    """Append an audit event to the JSONL log.

    Parameters
    ----------
    agent:
        Agent identifier such as ``meeting`` or ``knowledge``.
    event:
        Event name e.g. ``run`` or ``failure``.
    status:
        Outcome label (``success``, ``error`` 등).
    details:
        Optional metadata payload to merge into the audit entry.
    log_path:
        Optional override for the audit file location.
    """

    target = _ensure_path(log_path or DEFAULT_AUDIT_PATH)
    payload: Dict[str, Any] = {
        "timestamp": datetime.now(timezone.utc).isoformat(),
        "agent": agent,
        "event": event,
        "status": status,
    }
    if details:
        payload.update(details)
    try:
        with target.open("a", encoding="utf-8") as handle:
            handle.write(json.dumps(payload, ensure_ascii=False) + "\n")
    except Exception:
        # Audit logging should never break the main flow.
        pass


__all__ = ["record_event", "DEFAULT_AUDIT_PATH"]
