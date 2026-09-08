#!/bin/sh
# Oracle smoke (canary + cutover): TLS pin, STUN-lite echo, TURN 401, plus the
# external-mode invariant — the server must NEVER advertise a private address.
# Full bidirectional TURN (T1) needs a provisioned device (turn.credentials);
# that is Gate 9/15 on physical networks, not this script.
# Usage: smoke.sh <host[:port]> <fingerprint-hex> [expected-public-ip]
#   host: raw IP or hostname, optional :port (default 9091).
#     canary/cutover: <oracle-ipv4> (from HARBOR_ORACLE_ADDRESS); isolated: 127.0.0.1:19091
set -u
HOSTPORT="${1:?usage: smoke.sh <host[:port]> <fingerprint-hex> [expected-public-ip]}"
FINGERPRINT="${2:?usage: smoke.sh <host[:port]> <fingerprint-hex> [expected-public-ip]}"
EXPECTED="${3:-}"
PORT=9091
fail=0
say() { printf '%s\n' "$*"; }

case "$HOSTPORT" in
    \[*\]*)
        # [v6] or [v6]:port
        inner=${HOSTPORT#\[}; inner=${inner%\]*};
        rest=${HOSTPORT#*\]}
        case "$rest" in
            :*) PORT=${rest#:}; HOST="[$inner]" ;;
            *) HOST="[$inner]" ;;
        esac
        tls_target="$HOST:$PORT"; udp_host=$inner ;;
    *:*:*)
        # unbracketed IPv6, optional trailing :port
        case "$HOSTPORT" in
            *:9091|*:19091|*:[0-9]*)
                PORT=${HOSTPORT##*:}; HOST=${HOSTPORT%:*} ;;
            *) HOST=$HOSTPORT ;;
        esac
        tls_target="[$HOST]:$PORT"; udp_host=$HOST ;;
    *:*)
        HOST=${HOSTPORT%:*}; PORT=${HOSTPORT##*:}
        tls_target="$HOST:$PORT"; udp_host=$HOST ;;
    *) HOST="$HOSTPORT"; tls_target="$HOST:$PORT"; udp_host=$HOST ;;
esac

if ! command -v openssl >/dev/null 2>&1; then say "SKIP tcp: openssl missing"; fail=1;
else
    got=$(printf '' | openssl s_client -connect "$tls_target" -servername harbor 2>/dev/null \
        | openssl x509 -noout -fingerprint -sha256 2>/dev/null \
        | tr -d ':' | tr 'A-F' 'a-f' | cut -d= -f2)
    if [ "$got" = "$FINGERPRINT" ]; then say "OK   tcp: pinned certificate served";
    else say "FAIL tcp: fingerprint mismatch (got ${got:-nothing})"; fail=1; fi
fi

udp_probe() {
    # $1 = hex payload to send, $2 = expect substring in reply (empty = just hexdump first 4 bytes)
    python3 - "$udp_host" "$PORT" "$1" "$2" <<'EOF'
import socket, sys
host, port, hexdata, expect = sys.argv[1], int(sys.argv[2]), sys.argv[3], sys.argv[4]
data = bytes.fromhex(hexdata)
s = socket.socket(socket.AF_INET6 if ':' in host else socket.AF_INET, socket.SOCK_DGRAM)
s.settimeout(3)
try:
    s.sendto(data, (host, port))
    reply, _ = s.recvfrom(2048)
except Exception as e:
    print(f"TIMEOUT:{e}"); sys.exit(3)
if expect:
    sys.stdout.write(reply.decode('utf-8', 'replace'))
else:
    sys.stdout.write(reply[:4].hex())
EOF
}

STUN_HEX=$(printf '{"type":"stun_lite_request","nonce":"oracle-1"}' | od -An -tx1 | tr -d ' \n')
ALLOC_HEX="000300082112a4420000000100000002000000030019000411000000"

if command -v nc >/dev/null 2>&1; then
    reply=$(printf '{"type":"stun_lite_request","nonce":"oracle-1"}' \
        | nc -u -w 3 "$udp_host" "$PORT" 2>/dev/null | head -c 400)
    case "$reply" in
        *oracle-1*) say "OK   udp: STUN-lite echoed the probe" ;;
        *) say "FAIL udp: STUN-lite silent (got: ${reply:-nothing})"; fail=1 ;;
    esac
    first2=$(printf '\000\003\000\010\041\022\244\102\000\000\000\001\000\000\000\002\000\000\000\003\000\025\000\004\021\000\000\000' \
        | nc -u -w 3 "$udp_host" "$PORT" 2>/dev/null | od -An -tx1 -N4 2>/dev/null | tr -d ' \n')
    case "$first2" in
        0113000*|0113*) say "OK   udp: TURN challenged the bare Allocate" ;;
        *) say "FAIL udp: TURN silent (first bytes: ${first2:-none})"; fail=1 ;;
    esac
elif command -v python3 >/dev/null 2>&1; then
    reply=$(udp_probe "$STUN_HEX" "oracle-1" 2>/dev/null)
    case "$reply" in
        *oracle-1*) say "OK   udp: STUN-lite echoed the probe (python)" ;;
        *) say "FAIL udp: STUN-lite silent (got: ${reply:-nothing})"; fail=1 ;;
    esac
    first2=$(udp_probe "$ALLOC_HEX" "" 2>/dev/null)
    case "$first2" in
        0113000*|0113*) say "OK   udp: TURN challenged the bare Allocate (python)" ;;
        *) say "FAIL udp: TURN silent (first bytes: ${first2:-none})"; fail=1 ;;
    esac
else
    say "SKIP udp: nc and python3 missing"; fail=1
fi

# External-mode invariant reminder: XOR-RELAYED-ADDRESS must be dialable
# externally. A 401 challenge proves the state machine; the address value is
# proven by cargo tests + Gate 9 bidirectional media (never 10.0.0.7/wildcard).
if [ -n "$EXPECTED" ]; then
    say "INFO turn: expected public relay $EXPECTED — prove via Gate 9 media (cargo: external_allocate_advertises_public_ip_with_dedicated_socket)"
fi

[ "$fail" -eq 0 ] && say "smoke: all probes passed"
exit "$fail"
