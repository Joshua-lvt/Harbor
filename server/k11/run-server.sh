#!/data/data/com.termux/files/usr/bin/sh
# Canonical idempotent start of the Harbor control plane (dual-stack, the
# production bind from the runbook). Safe to run any number of times: a
# healthy listener is never touched, and a live-but-portless (wedged)
# process is stopped before rebinding.
set -u

PORT_HEX="2383"

port_listening() {
    # LISTEN is state 0A: a bare ":port" match also catches CLOSE/TIME_WAIT
    # leftovers of a dying process and would hide a real outage.
    grep ":$PORT_HEX" /proc/net/tcp /proc/net/tcp6 2>/dev/null | grep -q " 0A "
}

if port_listening; then
    exit 0
fi

for p in /proc/[0-9]*; do
    case "$(cat "$p/cmdline" 2>/dev/null | tr '\0' ' ')" in
        "$HOME/harbor/harbor-server"*)
            kill "${p#/proc/}" 2>/dev/null
            ;;
    esac
done
sleep 1
if port_listening; then
    exit 0
fi

HARBOR_SERVER_BIND="[::]:9091" \
HARBOR_SERVER_STATE_DIR="$HOME/harbor/state" \
    nohup "$HOME/harbor/harbor-server" >> "$HOME/harbor/server.log" 2>&1 &
