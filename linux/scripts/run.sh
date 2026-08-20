#!/usr/bin/env bash
# Convenience wrapper to run a production-built Harbor binary with rendering
# settings that avoid the webkit2gtk-on-Wayland protocol error seen under some
# compositors (Hyprland included). Uses X11/XWayland when present.
#
# Usage: ./scripts/run.sh
set -euo pipefail
cd "$(dirname "$0")/.."
APPIMAGE=""
for candidate in src-tauri/target/release/bundle/appimage/Harbor_*.AppImage; do
  if [[ -x "$candidate" ]]; then
    APPIMAGE="$candidate"
    break
  fi
done
BIN="src-tauri/target/release/harbor-app"

if [[ -n "$APPIMAGE" ]]; then
  BIN="$APPIMAGE"
fi

if [[ ! -x "$BIN" ]]; then
  echo "[run] release binary not found — run ./scripts/package.sh first."
  echo "[run] falling back to debug binary."
  BIN="src-tauri/target/debug/harbor-app"
fi

if [[ ! -x "$BIN" ]]; then
  echo "[run] no built binary found. Build with ./scripts/build.sh or ./scripts/package.sh" >&2
  exit 1
fi

# WebKitGTK on Hyprland can fail to allocate a GBM buffer even when the
# compositor exposes a valid Wayland session. Prefer XWayland automatically
# when it is available; set HARBOR_FORCE_WAYLAND=1 to opt out for testing.
if [[ "${HARBOR_FORCE_WAYLAND:-0}" != "1" && -n "${DISPLAY:-}" ]]; then
  export GDK_BACKEND=x11
  export WEBKIT_DISABLE_COMPOSITING_MODE=1
  export WEBKIT_DISABLE_DMABUF_RENDERER=1
fi

exec "$BIN" "$@"