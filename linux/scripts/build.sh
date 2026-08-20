#!/usr/bin/env bash
# Build + run Harbor Linux in development mode.
# - Installs frontend deps + Rust deps on first run.
# - Starts Vite (port 1420) and cargo (tauri dev).
#
# Usage: ./scripts/build.sh [--strict] [--release]
set -euo pipefail
cd "$(dirname "$0")/.."

# If npm deps aren't installed yet, install them (one-time).
if [[ ! -d node_modules ]]; then
  echo "[build] installing frontend deps…"
  npm install
fi

MODE=""
case "${1:-}" in
  --release|-r) MODE="--release" ;;
  --strict|-s)
    echo "[build] skipping dev run (compile check only)"
    cd src-tauri && cargo check
    exit $?
    ;;
esac

echo "[build] starting Harbor Linux dev server…"
exec cargo tauri dev $MODE