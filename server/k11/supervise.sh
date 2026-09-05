#!/data/data/com.termux/files/usr/bin/sh
# Harbor K11+ supervisor: keeps tailscaled + harbor-server alive across
# Android kills and network drops. Termux has no systemd, so this loop IS
# the supervisor. It is (re)started at boot by ~/.termux/boot/harbor (needs
# the Termux:Boot app) and manually after deploys. Every start path is
# idempotent: healthy listeners are never touched.
#
# What it deliberately does NOT do: re-login to Tailnet (the statedir keeps
# the node keys, so a fresh daemon re-establishes the mesh on its own), or
# change any client pin (the certificate fingerprint is transport-agnostic
# and never changes across restarts).
set -u

LOG="$HOME/harbor/supervisor.log"
TS_BIN="$HOME/tailscale_1.102.3_arm/tailscaled"
TS_STATE="$HOME/tailscale-state"
TS_SOCK="$HOME/tailscale.sock"
TS_LOG="$HOME/tailscale.log"
SERVER_LOG="$HOME/harbor/server.log"
PORT_HEX="2383"

log() {
    printf '%s %s\n' "$(date '+%F %T')" "$*" >> "$LOG"
}

# Fewer background kills while we supervise. Needs the Termux:API app;
# a missing command is harmless, so this never aborts the loop.
if command -v termux-wake-lock >/dev/null 2>&1; then
    termux-wake-lock
else
    log "termux-wake-lock unavailable (Termux:API not installed?)"
fi

# Rotate a log past 5 MiB, keeping one older generation.
rotate() {
    f="$1"
    [ -f "$f" ] || return 0
    size=$(wc -c < "$f" 2>/dev/null || echo 0)
    case "$size" in
        ''|*[!0-9]*) size=0 ;;
    esac
    if [ "$size" -gt 5242880 ]; then
        mv -f "$f" "$f.1"
        log "rotated $f ($size bytes)"
    fi
}

# True when a process with this exact cmdline prefix exists.
proc_running() {
    prefix="$1"
    for p in /proc/[0-9]*; do
        case "$(cat "$p/cmdline" 2>/dev/null | tr '\0' ' ')" in
            "$prefix"*) return 0 ;;
        esac
    done
    return 1
}

port_listening() {
    # LISTEN is state 0A: a bare ":port" match also catches CLOSE/TIME_WAIT
    # leftovers of a dying process and would hide a real outage.
    grep ":$PORT_HEX" /proc/net/tcp /proc/net/tcp6 2>/dev/null | grep -q " 0A "
}

ensure_tailscaled() {
    if proc_running "$TS_BIN"; then
        return 0
    fi
    # A stale socket blocks a fresh daemon: drop it only with no owner.
    rm -f "$TS_SOCK"
    log "tailscaled down: starting"
    "$TS_BIN" --tun=userspace-networking \
        --statedir="$TS_STATE" --socket="$TS_SOCK" \
        >> "$TS_LOG" 2>&1 &
}

ensure_server() {
    if port_listening; then
        return 0
    fi
    log "harbor-server port down: (re)starting"
    sh "$HOME/harbor/run-server.sh"
}

log "supervisor start (pid $$)"
while true; do
    rotate "$SERVER_LOG"
    rotate "$TS_LOG"
    rotate "$LOG"
    ensure_tailscaled
    ensure_server
    sleep 30
done
