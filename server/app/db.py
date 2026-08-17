"""Async SQLite storage for the relay.

Schema is intentionally tiny — three concerns: device identity/pairing,
presence/last-seen, and an offline message buffer (outbox). No ORM: aiosqlite
with Row mapping keeps the relay a single thin file.
"""
from __future__ import annotations

import sqlite3
import time
from contextlib import asynccontextmanager
from typing import Any

import aiosqlite

from . import config

SCHEMA = """
CREATE TABLE IF NOT EXISTS devices (
    id            TEXT PRIMARY KEY,
    secret        TEXT NOT NULL,
    pairing_code  TEXT UNIQUE,
    partner_id    TEXT,
    display_name  TEXT,
    public_key    TEXT,
    avatar        TEXT,
    presence      TEXT NOT NULL DEFAULT 'offline',
    last_seen     REAL,
    created_at    REAL NOT NULL
);
CREATE TABLE IF NOT EXISTS outbox (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    from_id      TEXT NOT NULL,
    to_id        TEXT NOT NULL,
    payload      TEXT NOT NULL,
    ts           REAL NOT NULL,
    delivered    INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_outbox_to   ON outbox(to_id, delivered);
CREATE INDEX IF NOT EXISTS idx_devices_code ON devices(pairing_code);
"""

_SELECT_DEVICE = (
    "SELECT id, secret, pairing_code, partner_id, display_name, public_key, avatar, presence, last_seen "
    "FROM devices WHERE id = ?"
)


@asynccontextmanager
async def connect():
    """Yield a short-lived aiosqlite connection with Row factory + commit/rollback."""
    conn = await aiosqlite.connect(config.DB_PATH)
    conn.row_factory = aiosqlite.Row
    try:
        await conn.execute("PRAGMA foreign_keys = ON")
        yield conn
        await conn.commit()
    except Exception:
        await conn.rollback()
        raise
    finally:
        await conn.close()


async def init_db() -> None:
    async with connect() as conn:
        await conn.executescript(SCHEMA)
        # Existing harbor.db files predate the `public_key` column (added for
        # E2E — see pairing.register_device / ws.py `enc` forwarding).
        # CREATE TABLE IF NOT EXISTS won't add a column to an existing table, so
        # backfill with an idempotent ALTER. SQLite has no ADD COLUMN IF NOT
        # EXISTS; it raises OperationalError "duplicate column name" on a fresh
        # DB where SCHEMA already created the column — swallow that, re-raise
        # anything else so a real schema error still surfaces. (NOT IntegrityError.)
        try:
            await conn.execute("ALTER TABLE devices ADD COLUMN public_key TEXT")
        except sqlite3.OperationalError as e:
            if "duplicate column" not in str(e):
                raise
        # `avatar` (profile photo, base64 JPEG data URL) added the same way —
        # backfill existing stores. Idempotent by the same OperationalError rule.
        try:
            await conn.execute("ALTER TABLE devices ADD COLUMN avatar TEXT")
        except sqlite3.OperationalError as e:
            if "duplicate column" not in str(e):
                raise


async def sweep_outbox() -> None:
    cutoff = time.time() - config.OFFLINE_MSG_TTL_DAYS * 86400
    async with connect() as conn:
        await conn.execute("DELETE FROM outbox WHERE ts < ?", (cutoff,))


async def get_device(conn: aiosqlite.Connection, device_id: str) -> dict[str, Any] | None:
    async with conn.execute(_SELECT_DEVICE, (device_id,)) as cur:
        row = await cur.fetchone()
        return dict(row) if row else None


async def find_by_code(conn: aiosqlite.Connection, code: str) -> str | None:
    async with conn.execute("SELECT id FROM devices WHERE pairing_code = ?", (code,)) as cur:
        row = await cur.fetchone()
        return row["id"] if row else None
