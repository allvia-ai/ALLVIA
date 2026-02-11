from __future__ import annotations

import hmac
import hashlib


def hmac_sha256(value: str, salt: str) -> str:
    key = salt.encode("utf-8")
    message = value.encode("utf-8")
    return hmac.new(key, message, hashlib.sha256).hexdigest()
