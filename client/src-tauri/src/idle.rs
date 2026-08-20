//! Idle detection: OS-wide seconds since last keyboard/mouse input.
//!
//! On Windows this calls `GetLastInputInfo` (via the `windows` crate) so "away"
//! reflects true machine idleness — not just whether a Harbor window has focus.
//! On Linux this uses X11 Screen Saver extension to query idle time.
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

#[cfg(target_os = "linux")]
fn idle_seconds_linux() -> u64 {
    // Try DBus first (works on GNOME, Mutter, and many Wayland compositors)
    if let Ok(idle) = get_idle_via_dbus() {
        return idle;
    }
    // Try Hyprland-specific method
    if let Ok(idle) = get_idle_via_hyprland() {
        return idle;
    }
    // Fallback to X11 Screen Saver extension
    if let Ok(idle) = get_idle_via_x11() {
        return idle;
    }
    // If all methods fail, return 0 (assume not idle)
    0
}

#[cfg(target_os = "linux")]
fn get_idle_via_dbus() -> Result<u64, Box<dyn std::error::Error>> {
    use zbus::blocking::{Connection, Proxy, ProxyBuilder};
    use std::ptr;
    use std::time::SystemTime;

    // Try to get the idle monitor from org.gnome.Mutter
    // This works on GNOME, and many other compositors that implement the Mutter IDLE interface
    let connection = Connection::session()?;
    let proxy: Proxy = ProxyBuilder::new(&connection)
            .interface("org.gnome.Mutter.IdleMonitor")?
            .path("/org/gnome/Mutter/IdleMonitor/Core")?
            .destination("org.gnome.Mutter.IdleMonitor")?
            .build()?;
    let response = proxy.call_method("GetIdletime", &())?;
    let idle_timestamp: u64 = response.body().deserialize()?;
    Ok(idle_timestamp / 1000)
}

#[cfg(target_os = "linux")]
fn get_idle_via_hyprland() -> Result<u64, Box<dyn std::error::Error>> {
    use std::process::Command;
    use std::str::FromStr;

    // Run hyprctl to get the idle_inactivity option
    let output = Command::new("hyprctl")
        .arg("getoption")
        .arg("idle_inactivity")
        .output()?;
    if !output.status.success() {
        return Err("hyprctl command failed".into());
    }
    let output_str = String::from_utf8_lossy(&output.stdout);
    let idle_ms: u64 = output_str.trim().parse()?;
    Ok(idle_ms / 1000)
}

#[cfg(target_os = "linux")]
fn get_idle_via_x11() -> Result<u64, Box<dyn std::error::Error>> {
    use x11_dl::{xlib, xss};

    // Load X11 library
    let xlib_match = xlib::Xlib::open();
    let xlib = match xlib_match {
        Ok(xlib) => xlib,
        Err(_) => return Err("Could not load X11 library".into()),
    };
    let xss_match = xss::Xss::open();
    let xss = match xss_match {
        Ok(xss) => xss,
        Err(_) => return Err("Could not load X11 Xss library".into()),
    };

    // Open connection to X server
    let display = unsafe { (xlib.XOpenDisplay)(std::ptr::null()) };
    if display.is_null() {
        return Err("Could not open X display".into());
    }

    let result = unsafe {
        // Get the root window
        let root = (xlib.XDefaultRootWindow)(display);

        // Query screensaver info for idle time
        let mut info = std::mem::MaybeUninit::<xss::XScreenSaverInfo>::uninit();

        let ret = (xss.XScreenSaverQueryInfo)(display, root, info.as_mut_ptr());
        (xlib.XCloseDisplay)(display);
        if ret != 0 {
            // XScreenSaverQueryInfo returns idle time in milliseconds
            let info = info.assume_init();
            Ok(info.idle as u64 / 1000)
        } else {
            Err("XScreenSaverQueryInfo failed".into())
        }
    };
    result
}

/// Seconds since the last keyboard/mouse input anywhere on the OS.
#[tauri::command]
pub fn get_idle_seconds() -> u64 {
    #[cfg(windows)]
    {
        idle_seconds_windows()
    }
    #[cfg(target_os = "linux")]
    {
        idle_seconds_linux()
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        0
    }
}
