from __future__ import annotations

import re

from core.data_pipeline.pipeline import TextCleaner

PII_PATTERNS = [
    re.compile(r"\d{3}-\d{2}-\d{4}"),
    re.compile(r"[A-Za-z0-9_.+-]+@[A-Za-z0-9-]+\.[A-Za-z0-9-.]+"),
]


def scrub_pii(text: str) -> str:
    cleaned = text
    for pattern in PII_PATTERNS:
        cleaned = pattern.sub("[PII]", cleaned)
    return TextCleaner.clean(cleaned)


def test_scrub_pii_masks_sensitive_data():
    sample = "Contact: 123-45-6789 or user@example.com"
    result = scrub_pii(sample)
    assert "[PII]" in result
    assert "example.com" not in result
