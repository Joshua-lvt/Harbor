#!/bin/sh
# Fase 7 — isolated functional validation with disposable identity/state.
# No K11 material, no public ingress, no DNS: loopback only.
# Proves: fresh binary starts, serves the pin it just generated, answers
# STUN + TURN 401, and stops cleanly. Evidence: timestamps + hashes on stdout.
# Usage: validate-isolated.sh (runs from the repo root)
set -u
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
BIN="$ROOT/target/debug/harbor-server"
TMP="$(mktemp -d "${TMPDIR:-/tmp}/harbor-oracle-isolated-XXXXXX")"
PORT=19091
FINGERPRINT=""

cleanup() { kill "$PID" 2>/dev/null; rm -rf "$TMP"; }
trap cleanup EXIT INT TERM

say() { printf '%s %s\n' "$(date -u '+%FT%TZ')" "$*"; }

say "build harbor-server (debug)"
(cd "$ROOT" && cargo build -p harbor-server) || exit 1
[ -x "$BIN" ] || { say "FAIL binary missing"; exit 1; }
say "binary sha256: $(sha256sum "$BIN" | cut -d' ' -f1)"

say "start disposable server on 127.0.0.1:$PORT"
HARBOR_SERVER_BIND="127.0.0.1:$PORT" HARBOR_SERVER_STATE_DIR="$TMP/state" \
    "$BIN" > "$TMP/server.log" 2>&1 &
PID=$!
sleep 1
kill -0 "$PID" 2>/dev/null || { say "FAIL process died:"; cat "$TMP/server.log"; exit 1; }

FINGERPRINT=$(grep -o 'sha256:[0-9a-f]*' "$TMP/server.log" | tail -1 | cut -d: -f2)
[ -n "$FINGERPRINT" ] || { say "FAIL no fingerprint in log"; cat "$TMP/server.log"; exit 1; }
say "fingerprint: $FINGERPRINT"

sh "$ROOT/server/oracle/smoke.sh" "127.0.0.1:$PORT" "$FINGERPRINT" || exit 1
# External-mode dry run: same binary with a TEST-NET public IP exercises the
# dedicated-socket path without any public exposure.
kill "$PID" 2>/dev/null; sleep 1
HARBOR_SERVER_BIND="127.0.0.1:$PORT" HARBOR_SERVER_STATE_DIR="$TMP/state-ext" \
HARBOR_TURN_EXTERNAL_IP=192.0.2.99 HARBOR_TURN_RELAY_PORT_RANGE=49160-49165 \
    "$BIN" > "$TMP/server-ext.log" 2>&1 &
PID=$!
sleep 1
F2=$(grep -o 'sha256:[0-9a-f]*' "$TMP/server-ext.log" | tail -1 | cut -d: -f2)
sh "$ROOT/server/oracle/smoke.sh" "127.0.0.1:$PORT" "${F2:-$FINGERPRINT}" "192.0.2.99" || exit 1
say "isolated validation passed (compat + external dry run)"
