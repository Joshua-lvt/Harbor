#!/data/data/com.termux/files/usr/bin/sh
# DDNS AAAA updater for the K11+ (Phase 4.2). Keeps a hostname's IPv6
# address in sync with whatever global address the carrier hands out,
# so clients pin "host:9091" once and never chase a rotating prefix.
#
# Provider-agnostic by design: you supply a URL template, this script
# supplies the address. Configure via environment (supervise.sh passes
# nothing — set these in ~/.termux/boot or the supervisor's environment):
#
#   HARBOR_DDNS_URL     required. '@AAA@' is replaced with the discovered
#                       global IPv6 (and '@A@' with IPv4, when present).
#                       Example (DuckDNS-style):
#                       https://example.duckdns.org/update?domains=example&token=SECRET&ipv6=@AAA@
#   HARBOR_DDNS_IFACE   optional. Prefer an address from this interface
#                       (e.g. rmnet_data0) instead of the kernel's default
#                       source choice. Stable SLAAC beats privacy addresses
#                       for inbound reachability.
#
# Secrets live only in the URL you set; nothing is committed here. The
# address never reaches the log — supervise.sh logs only ok/unchanged/failed.
set -u

[ -n "${HARBOR_DDNS_URL:-}" ] || { echo "ddns: HARBOR_DDNS_URL not set"; exit 1; }
command -v curl >/dev/null 2>&1 || { echo "ddns: curl not installed"; exit 1; }
command -v ip >/dev/null 2>&1 || { echo "ddns: ip not installed"; exit 1; }

# Discover the global IPv6. Preference order:
#   1. any global address on HARBOR_DDNS_IFACE (first wins — put the
#      interface whose address you want first)
#   2. the source address the kernel would use to reach the internet
#      (`ip -6 route get 2001:4860:4860::8888`) — correct for outbound,
#      which on a phone is usually the same prefix inbound.
# Temporary privacy addresses (Rnd hex) are skipped when a stable one
# exists, but Android often only has temporary ones — the runbook explains
# the tradeoff; inbound to a temporary address still works while it lives.
aaa=""
if [ -n "${HARBOR_DDNS_IFACE:-}" ]; then
    aaa=$(ip -6 -o addr show dev "$HARBOR_DDNS_IFACE" scope global 2>/dev/null \
        | sed -n 's/.* inet6 \([^ /]*\)\/.*/\1/p' | head -1)
fi
if [ -z "$aaa" ]; then
    aaa=$(ip -6 route get 2001:4860:4860::8888 2>/dev/null \
        | sed -n 's/.* src \([0-9a-f:]*\).*/\1/p' | head -1)
fi
[ -n "$aaa" ] || { echo "ddns: no global IPv6 found"; exit 1; }

a=$(ip -4 route get 8.8.8.8 2>/dev/null | sed -n 's/.* src \([0-9.]*\).*/\1/p' | head -1)

url=$(printf '%s' "$HARBOR_DDNS_URL" | sed "s|@AAA@|$aaa|; s|@A@|${a:-0.0.0.0}|")
# Feed curl's URL through stdin config so a credential-bearing provider URL
# never appears in /proc/<pid>/cmdline. Escape only config-file syntax.
escaped_url=$(printf '%s' "$url" | sed 's|\\|\\\\|g; s|"|\\"|g')

if out=$(printf 'url = "%s"\nsilent\nshow-error\nfail\nmax-time = 20\n' "$escaped_url" \
    | curl --config - 2>/dev/null); then
    case "$out" in
        KO|ko|ERROR|Error|error|*"badauth"*|*"not authorized"*|*"invalid token"*)
            echo "provider refused"; exit 1 ;;
        OK|ok|""|*"unchanged"*|*"already"*|*NO_CHANGE*|*nochange*)
            echo "unchanged or updated" ;;
        *) echo "updated" ;;
    esac
    exit 0
fi
echo "provider refused"
exit 1
