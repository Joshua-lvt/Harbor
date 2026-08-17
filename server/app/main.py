"""FastAPI application entrypoint.

Run (dev):  uvicorn app.main:app --host 0.0.0.0 --port 8000 --reload
Run (prod): uvicorn app.main:app --host 127.0.0.1 --port 8000  (behind a TLS terminator; --workers 1)
"""
from __future__ import annotations

from contextlib import asynccontextmanager

from fastapi import FastAPI
from fastapi.middleware.cors import CORSMiddleware

from . import db
from .routes import router as http_router
from .ws import router as ws_router


@asynccontextmanager
async def lifespan(_app: FastAPI):
    await db.init_db()
    await db.sweep_outbox()
    yield


app = FastAPI(title="Harbor Relay", version="0.1.0", lifespan=lifespan)

# CORS: the Tauri WebView loads the client from an origin different from the
# relay (dev: http://localhost:1420 / build: tauri://localhost or
# http://tauri.localhost), so the HTTP endpoints (/register, /pair, /profile,
# /partner) need Access-Control-Allow-Origin or the WebView2 blocks the fetch
# with "Failed to fetch". We allow every origin because the relay authenticates
# each mutation via the 256-bit `device_secret` in the request body — CORS is
# never the security boundary here (a private/operator-controlled relay is).
# Credentials are intentionally left disabled (default), which is what makes
# the "*" origin safe per the CORS spec. The WebSocket (/ws) is unaffected:
# WS handshakes carry an Origin only informatively and aren't subject to the
# same preflight that blocks fetch.
app.add_middleware(
    CORSMiddleware,
    allow_origins=["*"],
    allow_methods=["*"],
    allow_headers=["*"],
)

app.include_router(http_router)
app.include_router(ws_router)
