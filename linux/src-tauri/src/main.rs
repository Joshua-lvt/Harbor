// Prevents additional console window on Windows, keeps stdout on Linux.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use app_lib::run;

/// Apply WebKitGTK rendering workarounds BEFORE GTK/WebKit initializes.
///
/// WebKitGTK on some compositors (Hyprland included) fails to allocate a GBM
/// buffer even when the Wayland session looks valid, and the native Wayland
/// backend can throw a protocol error. The reliable workaround is to run on
/// XWayland (when available) with the DMA-BUF renderer and compositing mode
/// disabled. This mirrors `scripts/run.sh` so `npm run tauri dev` (which runs
/// the binary directly, bypassing the wrapper) gets the same behavior.
///
/// `HARBOR_FORCE_WAYLAND=1` opts out (for testing on a compositor that works
/// natively). The WebKit vars are harmless to set unconditionally; GDK_BACKEND
/// is only forced to x11 when an X display is actually present.
fn apply_webkit_workarounds() {
    // These two are safe to set always — they only affect WebKit's renderer.
    std::env::set_var("WEBKIT_DISABLE_COMPOSITING_MODE", "1");
    std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");

    // Prefer XWayland when available, unless the user explicitly opts out.
    let force_wayland = std::env::var("HARBOR_FORCE_WAYLAND").map_or(false, |v| v == "1");
    let has_display = std::env::var("DISPLAY").map_or(false, |v| !v.is_empty());
    if !force_wayland && has_display {
        std::env::set_var("GDK_BACKEND", "x11");
    }
}

fn main() {
    apply_webkit_workarounds();
    run();
}
