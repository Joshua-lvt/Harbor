#!/data/data/com.termux/files/usr/bin/sh
# Harbor K11+ supervisor: keeps harbor-server alive (control plane + STUN +
# TURN, all on 9091) across Android kills and network drops. Termux has no
# systemd, so this loop IS the supervisor. It is (re)started at boot by
# ~/.termux/boot/harbor (needs the Termux:Boot app) and manually after
# deploys. Every start path is idempotent: healthy listeners are never
# touched.
#
# What it deliberately does NOT do: change any client pin (the certificate
# fingerprint is transport-agnostic and never changes across restarts), or
# require root/TUN — the server is one dual-stack TCP+UDP socket.
#
# Optional DDNS (Phase 4.2): set HARBOR_DDNS_URL (see ddns-update.sh) and
# the loop refreshes the AAAA record every pass while it still serves.
set -u

LOG="$HOME/harbor/supervisor.log"
SERVER_LOG="$HOME/harbor/server.log"
PORT_HEX="2383"
DDNS_UPDATE="$HOME/harbor/ddns-update.sh"

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

tcp_listening() {
    # LISTEN is state 0A: a bare ":port" match also catches CLOSE/TIME_WAIT
    # leftovers of a dying process and would hide a real outage.
    grep ":$PORT_HEX" /proc/net/tcp /proc/net/tcp6 2>/dev/null | grep -q " 0A "
}

udp_bound() {
    # UDP has no LISTEN state and no TIME_WAIT leftovers: an entry with our
    # local port means a live socket owns it (STUN + TURN share it).
    grep -q ":$PORT_HEX " /proc/net/udp /proc/net/udp6 2>/dev/null
}

ensure_server() {
    if tcp_listening && udp_bound; then
        return 0
    fi
    if tcp_listening; then
        # TCP up but UDP gone: the process lost its datagram side. Kill it
        # outright — run-server.sh's own guard only looks at TCP and would
        # exit 0 without fixing anything.
        log "harbor-server TCP up but UDP 9091 missing: killing for restart"
        for p in /proc/[0-9]*; do
            case "$(cat "$p/cmdline" 2>/dev/null | tr '\0' ' ')" in
                "$HOME/harbor/harbor-server"*)
                    kill "${p#/proc/}" 2>/dev/null
                    ;;
            esac
        done
        sleep 1
    else
        log "harbor-server port down: (re)starting"
    fi
    sh "$HOME/harbor/run-server.sh"
}

ensure_ddns() {
    # Refresh the public AAAA while the server actually serves; a server
    # down this pass simply tries again next loop.
    [ -n "${HARBOR_DDNS_URL:-}" ] || return 0
    [ -x "$DDNS_UPDATE" ] || return 0
    tcp_listening || return 0
    if out=$("$DDNS_UPDATE" 2>&1); then
        case "$out" in
            *unchanged*) : ;;
            *) log "ddns: $out" ;;
        esac
    else
        log "ddns: update failed: $out"
    fi
}

log "supervisor start (pid $$)"
while true; do
    rotate "$SERVER_LOG"
    rotate "$LOG"
    ensure_server
    ensure_ddns
    sleep 30
done
