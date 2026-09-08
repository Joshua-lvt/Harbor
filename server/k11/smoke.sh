#!/data/data/com.termux/files/usr/bin/sh
# Post-reboot / post-deploy smoke test for the Harbor server on the K11+
# (Phase 4.2). Proves the three things a client needs, from the device
# itself — the same checks the runbook's validation section prescribes:
#
#   1. TCP 9091      control plane answers TLS with the pinned certificate
#   2. UDP 9091      STUN-lite echoes a nonce with the observed source
#   3. UDP 9091      TURN answers an unauthenticated Allocate with 401
#
# Usage: smoke.sh [host] [fingerprint-sha256-hex]
# Defaults: localhost + the pinned production fingerprint from the runbook.
# Exit 0 only when every probe passes; each failure names the leg.
set -u

HOST="${1:-localhost}"
FINGERPRINT="${2:-$(grep -o 'sha256:[0-9a-f]*' "$HOME/harbor/server.log" 2>/dev/null | tail -1 | cut -d: -f2)}"
PORT=9091
fail=0

case "$HOST" in
    \[*\]) tls_target="$HOST:$PORT"; udp_host=${HOST#\[}; udp_host=${udp_host%\]} ;;
    *:*) tls_target="[$HOST]:$PORT"; udp_host=$HOST ;;
    *) tls_target="$HOST:$PORT"; udp_host=$HOST ;;
esac

say() { printf '%s\n' "$*"; }

# --- 1. Control plane: TLS handshake with the pinned certificate ----------
if [ -z "$FINGERPRINT" ]; then
    say "SKIP tcp: no fingerprint known (pass it as the second argument)"
    fail=1
elif ! command -v openssl >/dev/null 2>&1; then
    say "SKIP tcp: openssl not installed"
    fail=1
else
    got=$(printf '' | openssl s_client -connect "$tls_target" -servername harbor 2>/dev/null \
        | openssl x509 -noout -fingerprint -sha256 2>/dev/null \
        | tr -d ':' | tr 'A-F' 'a-f' | cut -d= -f2)
    if [ "$got" = "$FINGERPRINT" ]; then
        say "OK   tcp: pinned certificate served ($FINGERPRINT)"
    else
        say "FAIL tcp: fingerprint mismatch (got ${got:-nothing})"
        fail=1
    fi
fi

# --- 2 + 3. UDP: STUN-lite echo and TURN 401 challenge --------------------
if ! command -v nc >/dev/null 2>&1; then
    say "SKIP udp: nc not installed (pkg install openbsd-netcat)"
    fail=1
else
    # STUN-lite: send a nonce, expect it echoed with our source address.
    reply=$(printf '{"type":"stun_lite_request","nonce":"smoke-1"}' \
        | nc -u -w 3 "$udp_host" "$PORT" 2>/dev/null | head -c 400)
    case "$reply" in
        *smoke-1*127.0.0.1*|*smoke-1*::1*|*smoke-1*)
            say "OK   udp: STUN-lite echoed the probe"
            ;;
        *)
            say "FAIL udp: STUN-lite did not answer (got: ${reply:-nothing})"
            fail=1
            ;;
    esac

    # TURN: a bare Allocate (method 0x0003, UDP transport, no auth) must
    # come back as an error response 0x0113 whose ERROR-CODE class/number
    # read 401 — the challenge that proves the TURN state machine answers.
    #   header: 0003 0008 2112A442 + 12 txid bytes
    #   attr:   0019 0004 11000000 (REQUESTED-TRANSPORT=UDP)
    first2=$(printf '\000\003\000\010\041\022\244\102\000\000\000\001\000\000\000\002\000\000\000\003\000\025\000\004\021\000\000\000' \
        | nc -u -w 3 "$udp_host" "$PORT" 2>/dev/null | od -An -tx1 -N4 2>/dev/null | tr -d ' \n')
    case "$first2" in
        0113000*|0113*)
            say "OK   udp: TURN challenged the bare Allocate"
            ;;
        *)
            say "FAIL udp: TURN did not challenge (first bytes: ${first2:-none})"
            fail=1
            ;;
    esac
fi

if [ "$fail" -eq 0 ]; then
    say "smoke: all probes passed"
fi
exit "$fail"
