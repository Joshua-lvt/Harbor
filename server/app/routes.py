"""HTTP routes: register / pair / profile / partner / unpair / me / health."""
from __future__ import annotations

import time

from fastapi import APIRouter, HTTPException

from . import models, pairing
from .ws import manager as ws_manager  # live sockets, for the `unpaired` push

router = APIRouter()


@router.get("/health")
async def health() -> dict:
    return {"status": "ok"}


@router.post("/register", response_model=models.RegisterResponse)
async def register(req: models.RegisterRequest) -> models.RegisterResponse:
    try:
        code, secret = await pairing.register_device(req.device_id, req.public_key, req.avatar)
    except pairing.PairError as e:
        raise HTTPException(status_code=e.status, detail=e.detail)
    return models.RegisterResponse(pairing_code=code, device_secret=secret)


@router.post("/pair", response_model=models.PairResponse)
async def pair(req: models.PairRequest) -> models.PairResponse:
    try:
        result = await pairing.pair(req.device_id, req.device_secret, req.partner_code)
    except pairing.PairError as e:
        raise HTTPException(status_code=e.status, detail=e.detail)
    return models.PairResponse(**result)


@router.post("/profile")
async def set_profile(req: models.UpdateProfileRequest) -> dict:
    try:
        return await pairing.set_profile(
            req.device_id,
            req.device_secret,
            req.display_name,
            req.public_key,
            req.avatar,
        )
    except pairing.PairError as e:
        raise HTTPException(status_code=e.status, detail=e.detail)


@router.get("/partner", response_model=models.PartnerInfo)
async def get_partner(device_id: str, secret: str) -> models.PartnerInfo:
    try:
        info = await pairing.get_partner(device_id, secret)
    except pairing.PairError as e:
        raise HTTPException(status_code=e.status, detail=e.detail)
    return models.PartnerInfo(**info)


@router.post("/unpair")
async def unpair(req: models.UnpairRequest) -> dict:
    """Break the caller's current pairing, bilaterally.

    Both devices get a freshly reissued pairing code (so each can pair with
    someone new) and the link is cleared. The ex-partner is notified over their
    live WebSocket (if any) with a ``{type: "unpaired", pairing_code}`` envelope
    carrying THEIR new code, so their client returns to the pairing screen
    immediately rather than clinging to a dead link. If the ex-partner is
    offline, they discover it on cold start via ``GET /partner`` (404 not paired)
    and resync with ``GET /me``.
    """
    try:
        code_me, partner_id, code_them = await pairing.unpair(req.device_id, req.device_secret)
    except pairing.PairError as e:
        raise HTTPException(status_code=e.status, detail=e.detail)
    if partner_id:
        await ws_manager.forward(
            partner_id,
            {"type": "unpaired", "pairing_code": code_them, "ts": time.time()},
        )
    return {"ok": True, "pairing_code": code_me}


@router.get("/me", response_model=models.MeInfo)
async def get_me(device_id: str, secret: str) -> models.MeInfo:
    """The caller's own state — current pairing code, partner link, display name.

    Used at cold start to recover a freshly reissued code after the partner
    unpaired us while we were offline (our local store still held the stale
    partner_id, but the relay already broke the link). Authenticated by the same
    (device_id, device_secret) pair as the WebSocket.
    """
    try:
        info = await pairing.get_me(device_id, secret)
    except pairing.PairError as e:
        raise HTTPException(status_code=e.status, detail=e.detail)
    return models.MeInfo(**info)
