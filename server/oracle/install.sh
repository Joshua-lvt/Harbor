#!/bin/sh
# Oracle host install for harbor-server (Fases 5-7). Idempotent; never touches
# the K11+ and never starts a second writable authority: staging runs on
# loopback with disposable state, production starts only via explicit
# --production after the Fase 16 snapshot is installed by hand.
# Usage: install.sh [--staging] [--check]
#   --staging  create user/dirs/unit, binary must exist at ./target/release/harbor-server
#   --check    verify only (permissions, unit syntax, binary present)
set -u
MODE="${1:---staging}"

fail() { printf 'install: FAIL %s\n' "$*" >&2; exit 1; }
say() { printf 'install: %s\n' "$*"; }

[ "$(id -u)" -eq 0 ] || fail "run as root on the Oracle guest"
command -v systemd-analyze >/dev/null 2>&1 || fail "systemd not found"

if [ "$MODE" = "--check" ]; then
    [ -f server/oracle/harbor-server.service ] || [ -f harbor-server.service ] || fail "unit file missing"
    systemd-analyze verify ./server/oracle/harbor-server.service 2>&1 \
        || systemd-analyze verify ./harbor-server.service 2>&1 \
        || fail "unit verification failed"
    say "unit syntax OK"
    [ -x /usr/local/bin/harbor-server ] || fail "binary missing at /usr/local/bin/harbor-server"
    say "binary present"
    exit 0
fi

[ "$MODE" = "--staging" ] || fail "usage: install.sh [--staging] [--check]"

BIN=""
for candidate in ./target/release/harbor-server ./harbor-server /tmp/harbor-server; do
    if [ -x "$candidate" ]; then BIN="$candidate"; break; fi
done
[ -n "$BIN" ] || fail "build first: cargo build --release -p harbor-server"

id harbor-server >/dev/null 2>&1 || useradd --system --no-create-home --shell /usr/sbin/nologin harbor-server
install -d -m 0700 -o harbor-server -g harbor-server /var/lib/harbor-server
install -d -m 0755 -o root -g root /etc/harbor-server
if [ ! -f /etc/harbor-server/harbor-server.env ]; then
    cp server/oracle/harbor-server.env.example /etc/harbor-server/harbor-server.env 2>/dev/null \
        || cp ./harbor-server.env.example /etc/harbor-server/harbor-server.env
    chmod 0600 /etc/harbor-server/harbor-server.env
    chown root:root /etc/harbor-server/harbor-server.env
    say "wrote default env (loopback staging: edit before production)"
fi
install -m 0755 "$BIN" /usr/local/bin/harbor-server
chown root:root /usr/local/bin/harbor-server
chmod 0755 /usr/local/bin/harbor-server
cp server/oracle/harbor-server.service /etc/systemd/system/harbor-server.service 2>/dev/null \
    || cp ./harbor-server.service /etc/systemd/system/harbor-server.service
systemd-analyze verify /etc/systemd/system/harbor-server.service || fail "unit verification failed"
systemctl daemon-reload
say "installed (unit disabled by default; staging: HARBOR_SERVER_BIND=127.0.0.1:9091 systemctl start harbor-server)"
