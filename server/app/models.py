"""Pydantic request/response schemas for the HTTP routes.

WebSocket envelopes are lightweight JSON dicts and are validated loosely in the
WS handler (see ws.py) rather than modelled here.
"""
from __future__ import annotations

from typing import Optional

from pydantic import BaseModel, Field


class RegisterRequest(BaseModel):
    device_id: str = Field(..., min_length=8)
    # Optional X25519 public key (base64). Set at register time so the relay can
    # hand it to a future partner via /pair + /partner. Optional so pre-E2E
    # clients / scripts that POST only device_id keep working.
    public_key: Optional[str] = None
    # Optional profile photo (base64 JPEG data URL). Published at register so a
    # future partner can render it; None/omitted keeps pre-avatar clients working.
    avatar: Optional[str] = None


class RegisterResponse(BaseModel):
    pairing_code: str
    device_secret: str


class PairRequest(BaseModel):
    device_id: str
    device_secret: str
    partner_code: str


class PairResponse(BaseModel):
    partner_device_id: str
    partner_name: Optional[str] = None
    # The partner's X25519 public key (base64), so the caller can seal messages
    # to them immediately. None if the partner hasn't published a key yet
    # (pre-E2E install / upgrade pending) — the client then falls back to
    # plaintext, visibly marked insecure.
    partner_public_key: Optional[str] = None
    # The partner's profile photo (base64 JPEG data URL). None if they haven't
    # set one yet — the client falls back to the shark mascot avatar.
    partner_avatar: Optional[str] = None


class UpdateProfileRequest(BaseModel):
    device_id: str
    device_secret: str
    display_name: Optional[str] = None
    # Publishing/updating the device's public key. This is the upgrade path for
    # already-paired installs that predate E2E (re-/register would re-issue the
    # device_secret and force a WS re-auth; /profile keeps the secret stable).
    public_key: Optional[str] = None
    # Publishing/updating the device's profile photo (base64 JPEG data URL).
    # An empty string "" clears it; None/omitted leaves the stored value as-is
    # (distinct from clearing), mirroring the public_key "don't touch" rule.
    avatar: Optional[str] = None


class UnpairRequest(BaseModel):
    device_id: str
    device_secret: str


class PartnerInfo(BaseModel):
    partner_device_id: str
    partner_name: Optional[str] = None
    presence: str
    last_seen: Optional[float] = None
    # The partner's X25519 public key (base64) — the passive partner retrieves
    # the caller's key here on cold start, since they weren't the one who called
    # /pair. None if the partner hasn't published a key yet.
    partner_public_key: Optional[str] = None
    # The partner's profile photo (base64 JPEG data URL) — retrieved on cold
    # start by the passive partner. None if they haven't set one yet.
    partner_avatar: Optional[str] = None


class MeInfo(BaseModel):
    pairing_code: Optional[str] = None
    partner_id: Optional[str] = None
    display_name: Optional[str] = None
