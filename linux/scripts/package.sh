#!/usr/bin/env bash
# Build a production Harbor Linux bundle (AppImage + .deb) into
# src-tauri/target/release/bundle/.
#
# Usage: ./scripts/package.sh [--debug] [--target appimage|deb|rpm]
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
REPO_ROOT="$(cd "$ROOT/.." && pwd)"
cd "$ROOT"

if [[ ! -d node_modules ]]; then
  echo "[package] installing frontend deps…"
  npm install
fi

TAURI_TARGETS="${1:-appimage,deb}"
echo "[package] building Harbor Linux for: $TAURI_TARGETS"

# Keep production packaging reproducible when the repository ships the
# linuxdeploy tools beside the project. Tauri's bundler honors LINUXDEPLOY;
# adding the GTK plugin directory to PATH lets linuxdeploy discover it without
# requiring a system-wide install.
LOCAL_LINUXDEPLOY="$REPO_ROOT/linuxdeploy-x86_64.AppImage"
LOCAL_TOOLS="$ROOT/.tools"
if [[ -x "$LOCAL_LINUXDEPLOY" ]]; then
  mkdir -p "$LOCAL_TOOLS"
  ln -sfn "$LOCAL_LINUXDEPLOY" "$LOCAL_TOOLS/linuxdeploy"
fi
if [[ -x "$REPO_ROOT/linuxdeploy-plugin-gtk/linuxdeploy-plugin-gtk.sh" ]]; then
  mkdir -p "$LOCAL_TOOLS"
  ln -sfn "$REPO_ROOT/linuxdeploy-plugin-gtk/linuxdeploy-plugin-gtk.sh" "$LOCAL_TOOLS/linuxdeploy-plugin-gtk"
fi
export PATH="$LOCAL_TOOLS:$PATH"

# Tauri reads --bundles for the target format list.
cd src-tauri
exec cargo tauri build --bundles "$TAURI_TARGETS"