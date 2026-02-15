"""Compiled regex patterns for PII masking."""
import re
from typing import Dict, Pattern

# Broad patterns to catch variations
EMAIL_PATTERN = re.compile(
    r"[a-zA-Z0-9._%+-]+(?:\s*@\s*|\s+at\s+)[a-zA-Z0-9.-]+\s*(?:\.|dot)\s*[a-zA-Z]{2,}",
    re.IGNORECASE,
)

# Korean mobile/landline variations
PHONE_PATTERN = re.compile(
    r"(?:010|02|0[3-6][1-5])\s*-?\s*[0-9]{3,4}\s*-?\s*[0-9]{4}",
)

# Resident Registration Number (7 digits masked)
RRN_PATTERN = re.compile(
    r"\d{6}\s*[-]\s*[1-4]\d{6}",
)

# Credit Card (16 digits)
CREDIT_CARD_PATTERN = re.compile(
    r"\d{4}\s*-?\s*\d{4}\s*-?\s*\d{4}\s*-?\s*\d{4}",
)

# Common IPv4 (just in case)
IP_PATTERN = re.compile(
    r"\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3}",
)

PATTERNS: Dict[str, Pattern] = {
    "EMAIL": EMAIL_PATTERN,
    "PHONE": PHONE_PATTERN,
    "RRN": RRN_PATTERN,
    "CREDIT_CARD": CREDIT_CARD_PATTERN,
    "IP": IP_PATTERN,
}

def mask_content(text: str) -> str:
    """Masks commonly identified PII in the text using regex."""
    masked_text = text
    for name, pattern in PATTERNS.items():
        masked_text = pattern.sub(f"[REDACTED_{name}]", masked_text)
    return masked_text
