//! Foreground-window detection + key-state polling (Push-to-Talk) for Linux.
//!
//! `get_foreground_app`: the foreground window's process, returned as a
//! `ForegroundApp { exe, path }`. `exe` is the lowercased basename
//! (e.g. "code") — the activity key + icon-cache key, privacy-respecting
//! (we report ONLY the process name, never the window title or screen
//! content). `path` is the full `/proc/[pid]/exe` result, used once per
//! new exe to extract the real icon (see `icons.rs`). The friendly name +
//! game detection happens client-side (`lib/appNames.ts`).
//!
//! `is_key_pressed`: whether a given virtual-key code is currently held. Used
//! by Push-to-Talk (default Left Alt, VK_LMENU = 0xA4) — polled with the X11
//! key query, which works under both native X11 and XWayland (Hyprland ships
//! XWayland, so PTT keeps working on Wayland through it).
//!
//! Strategies for foreground detection (tried in order):
//! 1. Hyprland-specific via `hyprctl activewindow -j` (Hyprland compositor)
//! 2. X11 via x11-dl (`_NET_ACTIVE_WINDOW` + `_NET_WM_PID`) — works under
//!    XWayland and native X11

use crate::icons::ForegroundApp;
use std::ptr;
use std::process::Command;
use std::fs::read_link;
use std::path::PathBuf;
use serde::Deserialize;

#[derive(Deserialize)]
struct HyprlandActiveWindow {
    pid: u32,
}

/// Foreground window's process: `{ exe, path }` (the exe basename is the
/// activity key; the full path is used once per exe to extract the icon).
/// Either field may be `None` on failure.
#[tauri::command]
pub fn get_foreground_app() -> Option<ForegroundApp> {
    // 1. Try Hyprland first (most reliable on Hyprland).
    if let Some(app) = foreground_app_via_hyprland() {
        return Some(app);
    }

    // 2. Fallback to X11 (works under XWayland in Hyprland, native X11).
    foreground_app_via_x11()
}

/// Get foreground app via Hyprland's `hyprctl` command.
fn foreground_app_via_hyprland() -> Option<ForegroundApp> {
    // Try JSON output first (more reliable parsing).
    let output = Command::new("hyprctl")
        .arg("activewindow")
        .arg("-j")
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let window: HyprlandActiveWindow = serde_json::from_slice(&output.stdout).ok()?;
    if window.pid == 0 {
        return None;
    }
    app_from_pid(window.pid)
}

/// Get foreground app via X11 (`_NET_ACTIVE_WINDOW` + `_NET_WM_PID`).
/// Works under XWayland (Hyprland ships XWayland) and native X11.
fn foreground_app_via_x11() -> Option<ForegroundApp> {
    use x11_dl::xlib;

    let xlib = xlib::Xlib::open().ok()?;

    unsafe {
        let display = (xlib.XOpenDisplay)(ptr::null());
        if display.is_null() {
            return None;
        }

        let root = (xlib.XDefaultRootWindow)(display);

        // Resolve _NET_ACTIVE_WINDOW on the root.
        let atom_active =
            (xlib.XInternAtom)(display, b"_NET_ACTIVE_WINDOW\0".as_ptr() as *const i8, 0);
        if atom_active == 0 {
            (xlib.XCloseDisplay)(display);
            return None;
        }

        let (active_window, ok) = read_cardinal_window(&xlib, display, root, atom_active);
        (xlib.XCloseDisplay)(display);
        if !ok || active_window == 0 {
            return None;
        }

        // Re-open for the per-window PID read (cheap).
        let display = (xlib.XOpenDisplay)(ptr::null());
        if display.is_null() {
            return None;
        }
        let atom_pid = (xlib.XInternAtom)(display, b"_NET_WM_PID\0".as_ptr() as *const i8, 0);
        let mut pid: u32 = 0;
        if atom_pid != 0 {
            let (v, got) = read_cardinal_u32(&xlib, display, active_window, atom_pid);
            if got {
                pid = v;
            }
        }
        (xlib.XCloseDisplay)(display);

        if pid == 0 {
            return None;
        }
        app_from_pid(pid)
    }
}

/// Read a `_NET_ACTIVE_WINDOW`-style atom (XA_WINDOW) from a window property.
/// Returns (value, ok).
unsafe fn read_cardinal_window(
    xlib: &x11_dl::xlib::Xlib,
    display: *mut x11_dl::xlib::_XDisplay,
    win: x11_dl::xlib::Window,
    atom: x11_dl::xlib::Atom,
) -> (x11_dl::xlib::Window, bool) {
    let mut atom_type: x11_dl::xlib::Atom = 0;
    let mut format: i32 = 0;
    let mut nitems: u64 = 0;
    let mut bytes_after: u64 = 0;
    let mut prop: *mut u8 = ptr::null_mut();

    let r = (xlib.XGetWindowProperty)(
        display,
        win,
        atom,
        0,
        1,
        0,
        x11_dl::xlib::XA_WINDOW,
        &mut atom_type,
        &mut format,
        &mut nitems,
        &mut bytes_after,
        &mut prop,
    );
    if r != x11_dl::xlib::Success as i32 || prop.is_null() || nitems == 0 {
        return (0, false);
    }
    let val = *(prop as *mut x11_dl::xlib::Window);
    (xlib.XFree)(prop as *mut _);
    (val, true)
}

/// Read a `_NET_WM_PID`-style atom (XA_CARDINAL) from a window property.
/// Returns (value, ok).
unsafe fn read_cardinal_u32(
    xlib: &x11_dl::xlib::Xlib,
    display: *mut x11_dl::xlib::_XDisplay,
    win: x11_dl::xlib::Window,
    atom: x11_dl::xlib::Atom,
) -> (u32, bool) {
    let mut atom_type: x11_dl::xlib::Atom = 0;
    let mut format: i32 = 0;
    let mut nitems: u64 = 0;
    let mut bytes_after: u64 = 0;
    let mut prop: *mut u8 = ptr::null_mut();

    let r = (xlib.XGetWindowProperty)(
        display,
        win,
        atom,
        0,
        1,
        0,
        x11_dl::xlib::XA_CARDINAL,
        &mut atom_type,
        &mut format,
        &mut nitems,
        &mut bytes_after,
        &mut prop,
    );
    if r != x11_dl::xlib::Success as i32 || prop.is_null() || format != 32 || nitems < 1 {
        return (0, false);
    }
    let val = *(prop as *mut u32);
    (xlib.XFree)(prop as *mut _);
    (val, true)
}

/// Given a PID, read `/proc/[pid]/exe` to get the executable path + basename.
fn app_from_pid(pid: u32) -> Option<ForegroundApp> {
    let exe_link = format!("/proc/{}/exe", pid);
    let path = read_link(&exe_link).ok()?;
    let path = path.to_string_lossy().to_string();
    let exe = PathBuf::from(&path)
        .file_name()
        .and_then(|s| s.to_str())
        .map(|s| s.to_lowercase());
    Some(ForegroundApp {
        exe,
        path: Some(path),
    })
}

/// Whether the given Windows virtual-key code is currently held.
/// Polls the X11 keymap (works under native X11 + XWayland on Hyprland).
#[tauri::command]
pub fn is_key_pressed(vk_code: u32) -> bool {
    is_key_pressed_impl(vk_code)
}

/// X11 key-state poll. Maps the Windows VK codes the frontend uses to X11
/// keysyms. Currently covers the PTT key (Left Alt) + a few neighbors; unknown
/// VKs return false so the mic stays muted rather than stuck open.
fn is_key_pressed_impl(vk: u32) -> bool {
    use x11_dl::xlib;

    let keysym = match vk {
        0x10 => 0xFFE1, // XK_Shift_L   (VK_SHIFT)
        0x11 => 0xFFE3, // XK_Control_L (VK_CONTROL)
        0x12 => 0xFFE9, // XK_Alt_L     (VK_MENU)
        0xA4 => 0xFFE9, // VK_LMENU     — Left Alt (Push-to-Talk default)
        0xA5 => 0xFFEA, // VK_RMENU     — Right Alt
        0x20 => 0x0020, // XK_space
        _ => return false, // unknown VK → mic stays muted (safe default)
    };

    let xlib = match xlib::Xlib::open() {
        Ok(x) => x,
        Err(_) => return false, // no X11 → assume key not pressed
    };

    unsafe {
        let display = (xlib.XOpenDisplay)(ptr::null());
        if display.is_null() {
            return false;
        }
        let keycode = (xlib.XKeysymToKeycode)(display, keysym);
        let pressed = if keycode == 0 {
            false
        } else {
            let mut keys: [i8; 32] = [0; 32];
            (xlib.XQueryKeymap)(display, keys.as_mut_ptr());
            let byte = (keycode / 8) as usize;
            let bit = (keycode % 8) as u8;
            byte < keys.len() && (keys[byte] & (1 << bit)) != 0
        };
        (xlib.XCloseDisplay)(display);
        pressed
    }
}

#[cfg(test)]
mod tests {
    use super::HyprlandActiveWindow;

    #[test]
    fn hyprland_window_json_deserializes_pid() {
        let json = br#"{"pid":1234,"class":"code","title":"editor"}"#;
        let window: HyprlandActiveWindow = serde_json::from_slice(json).unwrap();
        assert_eq!(window.pid, 1234);
    }

    #[test]
    fn hyprland_window_json_requires_pid() {
        let json = br#"{"class":"code"}"#;
        assert!(serde_json::from_slice::<HyprlandActiveWindow>(json).is_err());
    }
}
