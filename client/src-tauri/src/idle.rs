//! Idle detection: OS-wide seconds since last keyboard/mouse input.
//!
//! On Windows this calls `GetLastInputInfo` (via the `windows` crate) so "away"
//! reflects true machine idleness — not just whether a Harbor window has focus.
//! The frontend polls `get_idle_seconds` via a `#[tauri::command]`.

#[cfg(windows)]
fn idle_seconds_windows() -> u64 {
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
    use windows::Win32::System::SystemInformation::GetTickCount64;

    unsafe {
        let mut lii = LASTINPUTINFO {
            cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32,
            dwTime: 0,
        };
        // GetLastInputInfo returns BOOL; .as_bool() on the windows crate's BOOL.
        if GetLastInputInfo(&mut lii).as_bool() {
            // Both GetTickCount64 and LASTINPUTINFO.dwTime are milliseconds since boot.
            let now_ms = GetTickCount64();
            let last_ms = lii.dwTime as u64;
            if now_ms >= last_ms {
                (now_ms - last_ms) / 1000
            } else {
                0
            }
        } else {
            0
        }
    }
}

/// Seconds since the last keyboard/mouse input anywhere on the OS.
/// Returns 0 on non-Windows targets (Harbor targets Windows 11).
#[tauri::command]
pub fn get_idle_seconds() -> u64 {
    #[cfg(windows)]
    {
        idle_seconds_windows()
    }
    #[cfg(not(windows))]
    {
        0
    }
}
