"""Secret issuance and constant-time verification."""
from __future__ import annotations

import hmac
import secrets


def issue_secret() -> str:
    """A 256-bit URL-safe token kept by the client and presented on every WS connect."""
    return secrets.token_urlsafe(32)


def verify_secret(stored: str | None, presented: str | None) -> bool:
    if not stored or not presented:
        return False
    return hmac.compare_digest(stored, presented)
