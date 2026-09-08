#!/bin/sh
# Pre-write rollback (Fases 8/9/17): stop Oracle, remove test ingress, prove
# TCP+UDP 9091 closed externally. Safe only BEFORE Oracle accepts its first
# write. Post-write rollback = snapshot restore on the isolated K11 (see
# oracle-migration.md Fase 17): this script REFUSES post-write without
# --i-understand-post-write-needs-snapshot-restore plus the Oracle snapshot path.
# Usage: rollback.sh --pre-write | --i-understand-post-write-needs-snapshot-restore <snapshot-dir>
set -u
MODE="${1:-}"

if [ "$MODE" = "--pre-write" ]; then
    systemctl stop harbor-server 2>/dev/null || true
    systemctl disable harbor-server 2>/dev/null || true
    sh "$(dirname "$0")/firewall.sh" --close 2>/dev/null || true
    if command -v ss >/dev/null 2>&1 && ss -tlnup 2>/dev/null | grep -q ':9091 '; then
        echo "rollback: FAIL tcp 9091 still listening" >&2; exit 1
    fi
    echo "rollback: pre-write complete (Oracle stopped, 9091 ingress removed, K11 untouched authority)"
    exit 0
fi

if [ "$MODE" = "--i-understand-post-write-needs-snapshot-restore" ]; then
    echo "rollback: post-write requires the Fase 17 procedure (stop+isolate Oracle," >&2
    echo "copy its FINAL snapshot onto the stopped K11, validate hashes/pin, start" >&2
    echo "only K11, then return migrated clients via explicit server.configure" >&2
    echo "waves (no DNS name to repoint). Automating a" >&2
    echo "snapshot overwrite from this script would risk dual writers; refusing." >&2
    exit 2
fi

echo "usage: rollback.sh --pre-write" >&2
exit 2
