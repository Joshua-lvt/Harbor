"""Runtime configuration for the Harbor relay.

All values are environment-driven with safe defaults for local development.
The relay is intended to be run as a single private service for exactly one
paired couple — see the project README for the hosting philosophy.
"""
from __future__ import annotations

import os

# SQLite database file. Relative paths resolve against the process CWD.
DB_PATH: str = os.environ.get("HARBOR_DB_PATH", "harbor.db")

# How long (seconds) a connection may go silent before the server considers it
# dead. The client heartbeats well within this window.
HEARTBEAT_TIMEOUT_S: int = int(os.environ.get("HARBOR_HEARTBEAT_TIMEOUT_S", "90"))

# Delivered messages are deleted immediately; undelivered ones older than this
# are swept on startup (and could be swept on a schedule later).
OFFLINE_MSG_TTL_DAYS: int = int(os.environ.get("HARBOR_OFFLINE_MSG_TTL_DAYS", "7"))
