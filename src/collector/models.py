from __future__ import annotations

from dataclasses import dataclass, field
from datetime import datetime
from typing import Any, Dict, List, Optional

DEFAULT_SCHEMA_VERSION = "1.0"
VALID_PRIORITIES = {"P0", "P1", "P2"}


@dataclass
class ResourceRef:
    type: str
    id: str


@dataclass
class PrivacyMetadata:
    pii_level: str
    redaction: List[str] = field(default_factory=list)


@dataclass
class EventEnvelope:
    schema_version: str = DEFAULT_SCHEMA_VERSION
    event_id: str = ""
    ts: str = ""
    source: str = "unknown"
    app: str = "unknown"
    event_type: str = "unknown"
    priority: str = "P1"
    resource: ResourceRef = field(
        default_factory=lambda: ResourceRef(type="unknown", id="unknown")
    )
    payload: Dict[str, Any] = field(default_factory=dict)
    privacy: PrivacyMetadata = field(
        default_factory=lambda: PrivacyMetadata(pii_level="unknown")
    )
    pid: Optional[int] = None
    window_id: Optional[str] = None
    raw: Dict[str, Any] = field(default_factory=dict)


@dataclass
class SessionEvent:
    ts: datetime
    event_type: str
    priority: str
    app: str
    resource_type: str
    resource_id: str
    payload: Dict[str, Any]
