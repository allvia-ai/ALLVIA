from __future__ import annotations

import re
from typing import Iterable
from urllib.parse import urlparse

REDACTION_TOKEN = "[REDACTED]"


def truncate(value: str, max_len: int | None) -> str:
    if not max_len or max_len <= 0:
        return value
    if len(value) <= max_len:
        return value
    return value[:max_len]


def mask_patterns(value: str, patterns: Iterable[re.Pattern[str]]) -> str:
    masked = value
    for pattern in patterns:
        masked = pattern.sub(REDACTION_TOKEN, masked)
    return masked


def sanitize_url(value: str, keep_domain_only: bool = True) -> str:
    if not keep_domain_only:
        return value
    parsed = urlparse(value)
    if parsed.netloc:
        return parsed.netloc
    return value
