from __future__ import annotations

import datetime as dt
import uuid
from typing import Any, Dict, Optional, Tuple

from .models import (
    DEFAULT_SCHEMA_VERSION,
    VALID_PRIORITIES,
    EventEnvelope,
    PrivacyMetadata,
    ResourceRef,
)

SUPPORTED_MIN_VERSION = (1, 0)
SUPPORTED_MAX_VERSION = (1, 0)


class NormalizationError(ValueError):
    pass


def normalize_event(raw: Dict[str, Any], validation_level: str = "lenient") -> EventEnvelope:
    if not isinstance(raw, dict):
        raise NormalizationError("event must be an object")

    level = validation_level.strip().lower()
    if level not in {"lenient", "strict"}:
        raise NormalizationError(f"unknown validation level: {validation_level}")

    schema_version = str(raw.get("schema_version") or DEFAULT_SCHEMA_VERSION)
    version = _parse_version(schema_version)
    if version is None:
        if level == "strict":
            raise NormalizationError("invalid schema_version")
        schema_version = DEFAULT_SCHEMA_VERSION
        version = _parse_version(schema_version)

    compat_back = version is not None and version < SUPPORTED_MIN_VERSION
    compat_forward = version is not None and version > SUPPORTED_MAX_VERSION
    allow_missing_required = level == "lenient" or compat_back

    event_id = _normalize_event_id(raw.get("event_id"), allow_missing_required, level)

    ts = _normalize_ts(raw.get("ts"), allow_missing_required, level)

    source = _normalize_required_str(raw.get("source"), "source", allow_missing_required)
    app = _normalize_required_str(raw.get("app"), "app", allow_missing_required)
    event_type = _normalize_required_str(
        raw.get("event_type"), "event_type", allow_missing_required
    )
    priority = _normalize_priority(raw.get("priority"), allow_missing_required)

    resource = _normalize_resource(raw.get("resource"), allow_missing_required)
    payload = _normalize_payload(raw.get("payload"), allow_missing_required, level)
    privacy = _normalize_privacy(raw.get("privacy"), allow_missing_required, level)

    pid = _normalize_pid(raw.get("pid"))
    window_id = _normalize_window_id(raw.get("window_id"))

    if compat_forward and level == "strict":
        _ensure_required_fields_present(raw)

    return EventEnvelope(
        schema_version=schema_version,
        event_id=event_id,
        ts=ts,
        source=source,
        app=app,
        event_type=event_type,
        priority=priority,
        resource=resource,
        payload=payload,
        privacy=privacy,
        pid=pid,
        window_id=window_id,
        raw=raw,
    )


def _parse_version(value: str) -> Optional[Tuple[int, int]]:
    try:
        major_str, minor_str = value.split(".", 1)
        return int(major_str), int(minor_str)
    except (ValueError, AttributeError):
        return None


def _utc_now() -> str:
    return dt.datetime.utcnow().isoformat() + "Z"


def _normalize_ts(value: Any, allow_missing_required: bool, level: str) -> str:
    if value in (None, ""):
        if allow_missing_required:
            return _utc_now()
        raise NormalizationError("missing ts")
    if isinstance(value, str) and value:
        return value
    if level == "strict":
        raise NormalizationError("invalid ts")
    if isinstance(value, (int, float)):
        return dt.datetime.utcfromtimestamp(value).isoformat() + "Z"
    return _utc_now()


def _normalize_event_id(value: Any, allow_missing_required: bool, level: str) -> str:
    if not value:
        if allow_missing_required:
            return str(uuid.uuid4())
        raise NormalizationError("missing event_id")
    event_id = str(value)
    if level == "strict":
        try:
            uuid.UUID(event_id)
        except ValueError as exc:
            raise NormalizationError("invalid event_id") from exc
    return event_id


def _normalize_required_str(value: Any, name: str, allow_missing_required: bool) -> str:
    if value in (None, ""):
        if allow_missing_required:
            return "unknown"
        raise NormalizationError(f"missing {name}")
    if isinstance(value, str):
        return value
    return str(value)


def _normalize_priority(value: Any, allow_missing_required: bool) -> str:
    if not value:
        if allow_missing_required:
            return "P1"
        raise NormalizationError("missing priority")
    if value in VALID_PRIORITIES:
        return value
    if allow_missing_required:
        return "P1"
    raise NormalizationError("invalid priority")


def _normalize_payload(value: Any, allow_missing_required: bool, level: str) -> Dict[str, Any]:
    if value is None:
        if allow_missing_required:
            return {}
        raise NormalizationError("missing payload")
    if isinstance(value, dict):
        return value
    if level == "strict" and not allow_missing_required:
        raise NormalizationError("payload must be an object")
    return {}


def _normalize_resource(value: Any, allow_missing_required: bool) -> ResourceRef:
    if not isinstance(value, dict):
        if allow_missing_required:
            return ResourceRef(type="unknown", id="unknown")
        raise NormalizationError("missing resource")
    r_type = value.get("type")
    r_id = value.get("id")
    if r_type in (None, "") or r_id in (None, ""):
        if allow_missing_required:
            return ResourceRef(type="unknown", id="unknown")
        raise NormalizationError("invalid resource")
    return ResourceRef(type=str(r_type), id=str(r_id))


def _normalize_privacy(
    value: Any, allow_missing_required: bool, level: str
) -> PrivacyMetadata:
    if not isinstance(value, dict):
        if allow_missing_required:
            return PrivacyMetadata(pii_level="unknown", redaction=[])
        raise NormalizationError("missing privacy")
    pii_level = value.get("pii_level")
    if not pii_level:
        if allow_missing_required:
            pii_level = "unknown"
        else:
            raise NormalizationError("missing privacy.pii_level")
    redaction = value.get("redaction")
    if redaction is None:
        if level == "strict" and not allow_missing_required:
            raise NormalizationError("missing privacy.redaction")
        redaction_list = []
    elif isinstance(redaction, list):
        redaction_list = [str(item) for item in redaction]
    else:
        if level == "strict" and not allow_missing_required:
            raise NormalizationError("invalid privacy.redaction")
        redaction_list = []
    return PrivacyMetadata(pii_level=str(pii_level), redaction=redaction_list)


def _normalize_pid(value: Any) -> Optional[int]:
    if value is None:
        return None
    try:
        return int(value)
    except (TypeError, ValueError):
        return None


def _normalize_window_id(value: Any) -> Optional[str]:
    if value is None:
        return None
    if isinstance(value, (int, float)):
        return str(int(value))
    if isinstance(value, str) and value:
        return value
    return None


def _ensure_required_fields_present(raw: Dict[str, Any]) -> None:
    required = {
        "schema_version",
        "event_id",
        "ts",
        "source",
        "app",
        "event_type",
        "priority",
        "resource",
        "payload",
        "privacy",
    }
    missing = [key for key in required if key not in raw]
    if missing:
        raise NormalizationError("missing required fields: " + ", ".join(missing))
