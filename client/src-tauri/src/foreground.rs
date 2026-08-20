//! Foreground-window detection + key-state polling (Push-to-Talk).
//!
//! `get_foreground_app`: the foreground window's process, returned as a
//! `ForegroundApp { exe, path }`. `exe` is the lowercased basename
//! ("code.exe") — the activity key + icon-cache key, privacy-respecting (we
//! report ONLY the process name, never the window title or screen content).
//! `path` is the full `QueryFullProcessImageNameW` result, used once per new
//! exe to extract the real Windows icon (Feature 4; see `icons.rs`). On Linux,
//! we read `/proc/[pid]/exe` to get the executable path.
//! The friendly name + game detection happens client-side (`lib/appNames.ts`).
//!
//! `is_key_pressed`: whether a given virtual-key code is currently held. Used
//! by Push-to-Talk (default Left Alt) — polled with X11 key query on Linux.
//!
//! On Windows this uses Win32 APIs. On Linux this uses X11 and /proc.
//! On other platforms returns inert/default values.

use crate::icons::ForegroundApp;
use std::ptr;

#[cfg(windows)]
fn foreground_app_windows() -> Option<ForegroundApp> {
    use windows::core::PWSTR;
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{
        OpenProcess, PROCESS_NAME_FORMAT, PROCESS_QUERY_LIMITED_INFORMATION,
        QueryFullProcessImageNameW,
    };
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};

    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.0.is_null() {
            return None;
        }
        let mut pid: u32 = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == 0 {
            return None;
        }
        // PROCESS_QUERY_LIMITED_INFORMATION is the minimal access right — works
        // even on elevated processes (unlike PROCESS_QUERY_INFORMATION).
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 1024];
        let mut size = buf.len() as u32;
        QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_FORMAT(0),
            PWSTR(buf.as_mut_ptr()),
            &mut size,
        )
        .ok()?;
        let _ = CloseHandle(handle);
        let path = String::from_utf16_lossy(&buf[..size as usize]);
        // Report only the basename, lowercased (e.g. "vscode.exe"), for the
        // activity key — but keep the full path too for the icon extractor.
        let exe = path.rsplit('\\').next()?.to_lowercase();
        Some(ForegroundApp {
            exe: Some(exe),
            path: Some(path),
        })
    }
}

#[cfg(not(windows))]
fn foreground_app_windows() -> Option<ForegroundApp> {
    None
}

#[cfg(target_os = "linux")]
fn foreground_app_linux() -> Option<ForegroundApp> {
    // First, try to get the active window via Hyprland's hyprctl command
    if let Some(app) = get_foreground_app_via_hypraland() {
        return Some(app);
    }
    // Fallback to X11 method (works under XWayland in Hyprland)
    get_foreground_app_via_x11()
}

#[cfg(target_os = "linux")]
fn get_foreground_app_via_hypraland() -> Option<ForegroundApp> {
    use std::fs::read_link;
    use std::path::PathBuf;
    use std::process::Command;

    // Run hyprctl to get the active window in a parsable format
    // We try to get the JSON output first, which is more reliable
    let output = Command::new("hyprctl")
        .arg("activewindow")
        .arg("-j")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let output_str = String::from_utf8_lossy(&output.stdout);
    // Extract the PID from the JSON
    // We look for "pid":<number> in the JSON
    if let Some(pid_str) = output_str
        .split('\"')
        .nth(3) // Assuming the JSON structure: {"pid":1234,...} -> split by '"' gives [, pid, :, 1234, , ...]
        .and_then(|s| s.split(':').nth(1))
    {
        if let Ok(pid) = pid_str.trim().parse::<u32>() {
            // Now we have the PID, get the executable path from /proc
            let exe_path = format!("/proc/{}/exe", pid);
            let path = match read_link(&exe_path) {
                Ok(p) => p.to_string_lossy().to_string(),
                Err(_) => return None,
            };
            // Report only the basename, lowercased, for the activity key
            let exe = PathBuf::from(&path)
                .file_name()
                .and_then(|s| s.to_str())
                .map(|s| s.to_lowercase());
            return Some(ForegroundApp {
                exe,
                path: Some(path),
            });
        }
    }
    // If we couldn't parse the JSON, try to parse the non-JSON output as a fallback
    // This is less reliable but might work in some versions
    let output = Command::new("hyprctl")
        .arg("activewindow")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let output_str = String::from_utf8_lossy(&output.stdout);
    // Example output: "Window 0x12345678 'window title' (pid 1234): ..."
    if let Some(pid_str) = output_str
        .find("pid ")
        .and_then(|i| Some(output_str[i + 4..].chars().take_while(|c| c.is_ascii_digit()).collect::<String>())
        )
    {
        if let Ok(pid) = pid_str.parse::<u32>() {
            let exe_path = format!("/proc/{}/exe", pid);
            let path = match read_link(&exe_path) {
                Ok(p) => p.to_string_lossy().to_string(),
                Err(_) => return None,
            };
            let exe = PathBuf::from(&path)
                .file_name()
                .and_then(|s| s.to_str())
                .map(|s| s.to_lowercase());
            return Some(ForegroundApp {
                exe,
                path: Some(path),
            });
        }
    }
    None
}

#[cfg(target_os = "linux")]
fn get_foreground_app_via_x11() -> Option<ForegroundApp> {
    use std::fs::read_link;
    use std::path::PathBuf;
    use x11_dl::xlib;

    // Load X11 library
    let xlib = match xlib::Xlib::open() {
        Ok(xlib) => xlib,
        Err(_) => return None,
    };

    unsafe {
        // Open connection to X server
        let display = (xlib.XOpenDisplay)(ptr::null());
        if display.is_null() {
            return None;
        }

        // Get the active window
        let mut root_return: xlib::Window = 0;
        let mut child_return: xlib::Window = 0;
        let mut root_x: i32 = 0;
        let mut root_y: i32 = 0;
        let mut win_x: i32 = 0;
        let mut win_y: i32 = 0;
        let mut mask_return: u32 = 0;

        let root = (xlib.XDefaultRootWindow)(display);
        let result = (xlib.XQueryPointer)(
            display,
            root,
            &mut root_return,
            &mut child_return,
            &mut root_x,
            &mut root_y,
            &mut win_x,
            &mut win_y,
            &mut mask_return,
        );

        if result == 0 {
            (xlib.XCloseDisplay)(display);
            return None;
        }

        let active_window = if child_return != 0 { child_return } else { root_return };

        // Get the PID of the process owning the window
        let mut pid: u32 = 0;
        let atom_pid = (xlib.XInternAtom)(display, b"_NET_WM_PID\0".as_ptr() as *const i8, 0);
        if atom_pid != 0 {
            let mut atom_type: xlib::Atom = 0;
            let mut format: i32 = 0;
            let mut nitems: u64 = 0;
            let mut bytes_after: u64 = 0;
            let mut prop_return: *mut u8 = ptr::null_mut();

            let result = (xlib.XGetWindowProperty)(
                display,
                active_window,
                atom_pid,
                0,
                1,
                0,
                xlib::XA_CARDINAL,
                &mut atom_type,
                &mut format,
                &mut nitems,
                &mut bytes_after,
                &mut prop_return,
            );

            if result == xlib::Success as i32 && prop_return != ptr::null_mut() && format == 32 && nitems >= 1 {
                pid = *(prop_return as *mut u32);
                (xlib.XFree)(prop_return as *mut _);
            }
        }

        (xlib.XCloseDisplay)(display);

        if pid == 0 {
            return None;
        }

        // Get the executable path from /proc
        let exe_path = format!("/proc/{}/exe", pid);
        let path = match read_link(&exe_path) {
            Ok(p) => p.to_string_lossy().to_string(),
            Err(_) => return None,
        };

        // Report only the basename, lowercased, for the activity key
        let exe = PathBuf::from(&path)
            .file_name()
            .and_then(|s| s.to_str())
            .map(|s| s.to_lowercase());

        Some(ForegroundApp {
            exe,
            path: Some(path),
        })
    }
}

#[cfg(target_os = "linux")]
fn is_key_pressed_linux(vk: u32) -> bool {
    // Map common Windows VK codes to X11 keycodes
    // This is a simplified mapping - in practice you'd want a more complete mapping
    use x11_dl::xlib;

    // Load X11 library
    let xlib_match = xlib::Xlib::open();
    let xlib = match xlib_match {
        Ok(xlib) => xlib,
        Err(_) => return false, // If we can't load X11, assume key not pressed
    };

    // Map Windows VK to X11 keysym (partial mapping for common keys)
    let keysym = match vk {
        0x10 => 0xFFE1, // XK_Shift_L
        0x11 => 0xFFE3, // XK_Control_L
        0x12 => 0xFFE9, // XK_Alt_L
        0x20 => 0x0020, // XK_space
        _ => return false, // Unsupported key for now
    };

    unsafe {
        let display = (xlib.XOpenDisplay)(ptr::null());
        if display.is_null() {
            return false;
        }

        let keycode = (xlib.XKeysymToKeycode)(display, keysym);
        if keycode == 0 {
            (xlib.XCloseDisplay)(display);
            return false;
        }

        // Get the current key state
        let mut keys_return: [i8; 32] = [0; 32];
        (xlib.XQueryKeymap)(display, keys_return.as_mut_ptr());

        let byte_index = (keycode / 8) as usize;
        let bit_index = (keycode % 8) as u8;

        let is_pressed = if byte_index < keys_return.len() {
            (keys_return[byte_index] & (1 << bit_index)) != 0
        } else {
            false
        };

        (xlib.XCloseDisplay)(display);
        is_pressed
    }
}

/// Foreground window's process: `{ exe, path }` (the exe basename is the
/// activity key; the full path is used once per exe to extract the icon).
/// Either field may be `None` on failure.
#[tauri::command]
pub fn get_foreground_app() -> Option<ForegroundApp> {
    #[cfg(windows)]
    {
        foreground_app_windows()
    }
    #[cfg(target_os = "linux")]
    {
        foreground_app_linux()
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        None
    }
}

#[cfg(windows)]
fn is_key_pressed_windows(vk: u32) -> bool {
    use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
    unsafe {
        // High bit (0x8000) set => physically pressed (this process's message
        // queue). Minus the low bit repeating. Good enough for PTT polling.
        (GetAsyncKeyState(vk as i32) as u16 as u32 & 0x8000) != 0
    }
}

#[cfg(not(windows))]
fn is_key_pressed_windows(_vk: u32) -> bool {
    false
}

/// Whether the given Windows virtual-key code is currently held.
#[tauri::command]
pub fn is_key_pressed(vk_code: u32) -> bool {
    is_key_pressed_windows(vk_code)
}
