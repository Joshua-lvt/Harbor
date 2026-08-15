//! Foreground-window detection + key-state polling (Push-to-Talk).
//!
//! `get_foreground_app`: the foreground window's process, returned as a
//! `ForegroundApp { exe, path }`. `exe` is the lowercased basename
//! ("code.exe") — the activity key + icon-cache key, privacy-respecting (we
//! report ONLY the process name, never the window title or screen content).
//! `path` is the full `QueryFullProcessImageNameW` result, used once per new
//! exe to extract the real Windows icon (Feature 4; see `icons.rs`). The
//! friendly name + game detection happens client-side (`lib/appNames.ts`).
//!
//! `is_key_pressed`: whether a given virtual-key code is currently held. Used
//! by Push-to-Talk (default Left Alt) — polled cheaply with GetAsyncKeyState.
//!
//! Both return inert values on non-Windows targets (Harbor targets Win 11).

use crate::icons::ForegroundApp;

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

/// Foreground window's process: `{ exe, path }` (the exe basename is the
/// activity key; the full path is used once per exe to extract the icon).
/// Either field may be `None` on failure.
#[tauri::command]
pub fn get_foreground_app() -> Option<ForegroundApp> {
    foreground_app_windows()
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
