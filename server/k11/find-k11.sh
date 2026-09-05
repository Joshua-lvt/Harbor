#!/usr/bin/env bash
# Find the K11+'s current LAN address after a DHCP change.
#
# The K11+ gets a dynamic LAN lease, so a recorded 192.168.x.y goes stale
# whenever the Wi-Fi drops and returns. This probes the local LAN for the
# two Harbor facts that identify it: a Termux sshd banner on 8022 and the
# pinned control-plane certificate on 9091. The Tailnet endpoint needs no
# discovery (100.114.220.46 is stable) — use this only for LAN/SSH access.
#
#   ./find-k11.sh                       # scan neighbors on local /24s
#   ./find-k11.sh --subnet 192.168.1.0/24
#   HARBOR_FINGERPRINT=<64hex> ./find-k11.sh
#
# Dependencies: bash, ip, openssl, timeout. No nmap, no root.
set -euo pipefail

FINGERPRINT="${HARBOR_FINGERPRINT:-b9846aed2e97bd741ae5a2a3de9ab37c1831d2372ca67f26f538bd279dd7271f}"
SUBNET=""
TIMEOUT_S=3

while [ $# -gt 0 ]; do
    case "$1" in
        --subnet) SUBNET="$2"; shift 2 ;;
        --pin) FINGERPRINT="$2"; shift 2 ;;
        -h|--help) sed -n '2,16p' "$0"; exit 0 ;;
        *) echo "usage: $0 [--subnet 192.168.1.0/24] [--pin <64hex>]" >&2; exit 1 ;;
    esac
done

candidates() {
    if [ -n "$SUBNET" ]; then
        base="${SUBNET%.*}"
        for i in $(seq 1 254); do echo "$base.$i"; done
    else
        # Live ARP neighbors across all up interfaces: fast, no sweep.
        ip -o -4 neigh show 2>/dev/null | awk '{print $1}' | sort -u
    fi
}

probe() {
    ip="$1"
    ssh_banner=""
    # ssh-keyscan speaks just enough protocol for the banner; a raw socket
    # can stall on this sshd (reverse-DNS wait), so it is only a fallback.
    if command -v ssh-keyscan >/dev/null 2>&1; then
        ssh_banner=$(timeout "$TIMEOUT_S" ssh-keyscan -p 8022 -T "$TIMEOUT_S" -t rsa "$ip" 2>/dev/null \
            | grep -m1 -o 'SSH-2.0-[^ ]*' || true)
    else
        if out=$(timeout "$TIMEOUT_S" bash -c "exec 3<>/dev/tcp/$ip/8022 && head -c 40 <&3" 2>/dev/null); then
            ssh_banner="$out"
        fi
    fi
    pin=""
    # DER is binary: never store it in a shell variable (NUL bytes), pipe
    # straight into the hash so only text crosses the substitution.
    pin=$(echo | timeout "$TIMEOUT_S" openssl s_client -connect "$ip:9091" -showcerts 2>/dev/null \
        | openssl x509 -outform der 2>/dev/null | sha256sum 2>/dev/null | awk '{print $1}' || true)
    verdict="no"
    if [ "$pin" = "$FINGERPRINT" ]; then
        verdict="K11+ (pin match)"
    elif [[ "$ssh_banner" == SSH-2.0-OpenSSH* ]]; then
        verdict="possible (ssh only)"
    fi
    printf '%-16s %-18s %s\n' "$ip" "$verdict" "$ssh_banner"
}

export -f probe
export FINGERPRINT TIMEOUT_S

echo "probing LAN for the K11+ (pin ${FINGERPRINT:0:12}...) ..."
if [ -n "$SUBNET" ]; then
    candidates | xargs -P 64 -I{} bash -c 'probe "$@"' _ {} | sort -t. -k4 -n
else
    # Refresh ARP first so a just-returned K11+ shows up.
    for iface in $(ip -o -4 addr show up 2>/dev/null | awk '{print $2}' | sort -u); do
        addr=$(ip -o -4 addr show dev "$iface" 2>/dev/null | awk '{print $4}' | head -1)
        [ -z "$addr" ] && continue
        case "$addr" in 127.*|100.*) continue ;; esac
        bcast=$(ip -o -4 addr show dev "$iface" 2>/dev/null | awk '{print $6}' | head -1)
        [ -n "$bcast" ] && ping -b -c2 -W1 "$bcast" >/dev/null 2>&1 || true
    done
    candidates | xargs -P 32 -I{} bash -c 'probe "$@"' _ {} | sort -t. -k4 -n
fi
echo "done. SSH stays LAN-only: ssh -p 8022 joshy@<ip>."
