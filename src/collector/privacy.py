from __future__ import annotations

import re
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Dict, Iterable, List, Optional, Set

import yaml

from .models import EventEnvelope, PrivacyMetadata, ResourceRef

try:
    from .observability import Observability
except ImportError:  # pragma: no cover - optional for test import order
    Observability = None  # type: ignore
from .utils.hashing import hmac_sha256
from .utils.masking import mask_patterns, sanitize_url, truncate

EMAIL_KEYS = {
    "recipients",
    "recipient",
    "to",
    "cc",
    "bcc",
    "email",
    "emails",
}

EMAIL_PATTERN = re.compile(r"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}")


@dataclass
class PrivacyRules:
    mask_keys: Set[str] = field(default_factory=set)
    hash_keys: Set[str] = field(default_factory=set)
    length_limits: Dict[str, int] = field(default_factory=dict)
    url_policy: Dict[str, Any] = field(default_factory=dict)
    redaction_patterns: List[re.Pattern[str]] = field(default_factory=list)
    drop_payload_keys: Set[str] = field(default_factory=set)
    allowlist_apps: Set[str] = field(default_factory=set)
    denylist_apps: Set[str] = field(default_factory=set)
    denylist_action: str = "drop"


class PrivacyGuard:
    def __init__(
        self,
        rules: PrivacyRules,
        hash_salt: str,
        metrics: Optional["Observability"] = None,
    ) -> None:
        self._rules = rules
        self._hash_salt = hash_salt
        self._metrics = metrics

    def apply(self, envelope: EventEnvelope) -> Optional[EventEnvelope]:
        app_key = (envelope.app or "").lower()
        if self._rules.allowlist_apps and app_key not in self._rules.allowlist_apps:
            if self._metrics:
                self._metrics.record_drop("allowlist")
            return None
        if self._rules.denylist_apps and app_key in self._rules.denylist_apps:
            if self._rules.denylist_action == "strip":
                envelope.payload = {}
                envelope.privacy.redaction = _dedupe(
                    envelope.privacy.redaction + ["denylist_stripped"]
                )
                if self._metrics:
                    self._metrics.record_privacy_denied()
                return envelope
            if self._metrics:
                self._metrics.record_privacy_denied()
            return None

        redactions = list(envelope.privacy.redaction)

        if envelope.window_id:
            envelope.window_id = _hash_value(str(envelope.window_id), self._hash_salt)
            redactions.append("window_id_hashed")

        if envelope.resource and envelope.resource.id and envelope.resource.id != "unknown":
            envelope.resource = ResourceRef(
                type=envelope.resource.type,
                id=_hash_value(str(envelope.resource.id), self._hash_salt),
            )
            redactions.append("resource_id_hashed")

        sanitized: Dict[str, Any] = {}
        for key, value in envelope.payload.items():
            key_norm = key.lower()
            if key_norm in self._rules.drop_payload_keys:
                redactions.append(f"drop:{key_norm}")
                continue
            if key_norm in EMAIL_KEYS:
                sanitized[key] = _summarize_recipients(value)
                redactions.append(f"recipients_summarized:{key_norm}")
                continue
            sanitized[key] = self._sanitize_payload_value(key, value, redactions)

        envelope.payload = sanitized
        envelope.privacy = PrivacyMetadata(
            pii_level=envelope.privacy.pii_level,
            redaction=_dedupe(redactions),
        )
        if self._metrics and redactions:
            self._metrics.record_privacy_redacted()
        return envelope

    def _sanitize_payload_value(
        self, key: str, value: Any, redactions: List[str]
    ) -> Any:
        key_norm = key.lower()

        if key_norm in self._rules.hash_keys:
            redactions.append(f"hash:{key_norm}")
            return _hash_value(str(value), self._hash_salt)

        if isinstance(value, str):
            if key_norm == "url":
                allow_full = bool(self._rules.url_policy.get("allow_full_url", False))
                keep_domain_only = bool(
                    self._rules.url_policy.get("keep_domain_only", True)
                )
                if not allow_full:
                    value = sanitize_url(value, keep_domain_only=keep_domain_only)
                    redactions.append("url_sanitized")

            if key_norm in self._rules.mask_keys:
                value = mask_patterns(value, self._rules.redaction_patterns)
                redactions.append(f"mask:{key_norm}")

            max_len = self._rules.length_limits.get(key_norm)
            if max_len:
                value = truncate(value, max_len)

        return value


def load_privacy_rules(path: str | Path) -> PrivacyRules:
    path = Path(path)
    raw = yaml.safe_load(path.read_text()) or {}
    if not isinstance(raw, dict):
        raise ValueError("privacy rules must be a mapping")

    mask_keys = _lower_set(raw.get("mask_keys"))
    hash_keys = _lower_set(raw.get("hash_keys"))
    drop_payload_keys = _lower_set(raw.get("drop_payload_keys"))
    allowlist_apps = _lower_set(raw.get("allowlist_apps"))
    denylist_apps = _lower_set(raw.get("denylist_apps"))
    denylist_action = str(raw.get("denylist_action", "drop")).lower()

    length_limits = {
        str(k).lower(): int(v)
        for k, v in _as_dict(raw.get("length_limits")).items()
        if v is not None
    }

    url_policy = _as_dict(raw.get("url_policy"))

    redaction_patterns = []
    for pattern in raw.get("redaction_patterns", []) or []:
        if isinstance(pattern, dict):
            regex = pattern.get("regex")
        else:
            regex = pattern
        if not regex:
            continue
        redaction_patterns.append(re.compile(str(regex)))

    return PrivacyRules(
        mask_keys=mask_keys,
        hash_keys=hash_keys,
        length_limits=length_limits,
        url_policy=url_policy,
        redaction_patterns=redaction_patterns,
        drop_payload_keys=drop_payload_keys,
        allowlist_apps=allowlist_apps,
        denylist_apps=denylist_apps,
        denylist_action=denylist_action,
    )


def _as_dict(value: Any) -> Dict[str, Any]:
    if isinstance(value, dict):
        return value
    return {}


def _lower_set(value: Any) -> Set[str]:
    if not value:
        return set()
    if isinstance(value, (list, set, tuple)):
        return {str(item).lower() for item in value}
    return {str(value).lower()}


def _hash_value(value: str, salt: str) -> str:
    return hmac_sha256(value, salt)


def _dedupe(values: Iterable[str]) -> List[str]:
    seen = set()
    output = []
    for item in values:
        if item in seen:
            continue
        seen.add(item)
        output.append(item)
    return output


def _summarize_recipients(value: Any) -> Dict[str, Any]:
    emails = _collect_emails(value)
    if emails:
        domain_stats: Dict[str, int] = {}
        for email in emails:
            domain = _extract_domain(email)
            if not domain:
                continue
            domain_stats[domain] = domain_stats.get(domain, 0) + 1
        summary: Dict[str, Any] = {"count": len(emails)}
        if domain_stats:
            summary["domain_stats"] = domain_stats
        return summary

    count = _coerce_recipient_count(value)
    if count is None:
        return {"count": 0}
    return {"count": count}


def _collect_emails(value: Any) -> List[str]:
    emails: List[str] = []
    if isinstance(value, str):
        emails.extend(EMAIL_PATTERN.findall(value))
        return emails
    if isinstance(value, dict):
        for item in value.values():
            emails.extend(_collect_emails(item))
        return emails
    if isinstance(value, (list, tuple, set)):
        for item in value:
            emails.extend(_collect_emails(item))
        return emails
    return emails


def _extract_domain(email: str) -> str:
    parts = email.lower().split("@", 1)
    if len(parts) != 2:
        return ""
    return parts[1].strip()


def _coerce_recipient_count(value: Any) -> Optional[int]:
    if isinstance(value, (int, float)):
        return int(value)
    if isinstance(value, str):
        return 1 if value.strip() else None
    if isinstance(value, dict):
        count = value.get("count")
        if isinstance(count, (int, float)):
            return int(count)
    if isinstance(value, (list, tuple, set)):
        return len(value)
    return None
