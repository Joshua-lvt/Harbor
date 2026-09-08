#!/bin/sh
# Oracle guest firewall staging (Fase 8). firewalld, public zone, explicit
# per-protocol rules. 3478/5349 and the relay range stay CLOSED until Gate 9.
# Usage:
#   firewall.sh --staging 10.0.0.0/8...  allow 9091/tcp+udp from test sources only
#   firewall.sh --open-public             allow 9091/tcp+udp from anywhere (canary window only)
#   firewall.sh --close                   remove all harbor 9091 rules (rollback)
# Never restricts SSH here: prove a second admin path first (migration doc).
set -u
MODE="${1:---staging}"
shift 2>/dev/null || true

have() { command -v "$1" >/dev/null 2>&1; }
if ! have firewall-cmd; then echo "firewall: firewall-cmd missing" >&2; exit 2; fi

close_all() {
    firewall-cmd --permanent --remove-service=harbor-9091-tcp 2>/dev/null || true
    firewall-cmd --permanent --remove-port=9091/tcp 2>/dev/null || true
    firewall-cmd --permanent --remove-port=9091/udp 2>/dev/null || true
    for src in $(firewall-cmd --permanent --list-rich-rules 2>/dev/null | grep -o 'source address="[^"]*"' | cut -d'"' -f2); do
        firewall-cmd --permanent --remove-rich-rule="rule family=ipv4 source address=$src port port=9091 protocol=tcp accept" 2>/dev/null || true
        firewall-cmd --permanent --remove-rich-rule="rule family=ipv4 source address=$src port port=9091 protocol=udp accept" 2>/dev/null || true
    done
    firewall-cmd --reload
    echo "firewall: 9091 rules removed; verify with: ss -tlnup | grep 9091"
}

case "$MODE" in
    --close) close_all; exit 0 ;;
    --open-public)
        firewall-cmd --permanent --add-port=9091/tcp
        firewall-cmd --permanent --add-port=9091/udp
        firewall-cmd --reload
        echo "firewall: 9091/tcp+udp open (canary window ONLY; close immediately after)"
        exit 0
        ;;
    --staging)
        [ "$#" -gt 0 ] || { echo "usage: firewall.sh --staging <test-source-cidr>..." >&2; exit 2; }
        close_all >/dev/null
        for src in "$@"; do
            firewall-cmd --permanent --add-rich-rule="rule family=ipv4 source address=$src port port=9091 protocol=tcp accept"
            firewall-cmd --permanent --add-rich-rule="rule family=ipv4 source address=$src port port=9091 protocol=udp accept"
        done
        firewall-cmd --reload
        echo "firewall: 9091/tcp+udp restricted to: $*"
        echo "relay range 49160-49175/udp stays CLOSED until Gate 9"
        exit 0
        ;;
    *) echo "usage: firewall.sh (--staging <cidr>...|--open-public|--close)" >&2; exit 2 ;;
esac
