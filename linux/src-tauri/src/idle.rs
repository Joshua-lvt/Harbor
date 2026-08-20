//! Idle detection: OS-wide seconds since last keyboard/mouse input on Linux.
//!
//! Strategies tried in order of preference:
//! 1. DBus — GNOME/Mutter `org.gnome.Mutter.IdleMonitor.GetIdletime` (GNOME +
//!    compatible Wayland compositors).
//! 2. DBus — KDE Plasma `org.kde.ScreenSaver.GetSessionIdleTime`.
//! 3. systemd-logind — `loginctl show-session` (works from Wayland sessions,
//!    including Hyprland, when logind exposes monotonic idle timestamps).
//! 4. X11 Screen Saver extension (XSS) — works on native X11 and XWayland.
//!
//! The frontend polls `get_idle_seconds` via the `#[tauri::command]`.

use std::ptr;
use std::process::Command;

/// Seconds since the last keyboard/mouse input anywhere on the OS.
#[tauri::command]
pub fn get_idle_seconds() -> u64 {
    #[cfg(target_os = "linux")]
    {
        get_idle_seconds_linux()
    }
    #[cfg(not(target_os = "linux"))]
    {
        0
    }
}

#[cfg(target_os = "linux")]
fn get_idle_seconds_linux() -> u64 {
    if let Ok(idle) = get_idle_via_dbus_gnome() {
        return idle;
    }
    if let Ok(idle) = get_idle_via_dbus_kde() {
        return idle;
    }
    if let Ok(idle) = get_idle_via_logind() {
        return idle;
    }
    if let Ok(idle) = get_idle_via_x11() {
        return idle;
    }
    0
}

/// GNOME / Mutter IdleMonitor. Returns microseconds since last input.
#[cfg(target_os = "linux")]
fn get_idle_via_dbus_gnome() -> Result<u64, Box<dyn std::error::Error>> {
    use zbus::blocking::{Connection, Proxy, ProxyBuilder};

    let connection = Connection::session()?;
    let proxy: Proxy = ProxyBuilder::new(&connection)
        .interface("org.gnome.Mutter.IdleMonitor")?
        .path("/org/gnome/Mutter/IdleMonitor/Core")?
        .destination("org.gnome.Mutter.IdleMonitor")?
        .build()?;

    let response = proxy.call_method("GetIdletime", &())?;
    let idle_us: u64 = response.body().deserialize()?;
    Ok(idle_us / 1_000_000)
}

/// KDE Plasma ScreenSaver. `GetSessionIdleTime` returns milliseconds.
#[cfg(target_os = "linux")]
fn get_idle_via_dbus_kde() -> Result<u64, Box<dyn std::error::Error>> {
    use zbus::blocking::{Connection, Proxy, ProxyBuilder};

    let connection = Connection::session()?;
    let proxy: Proxy = ProxyBuilder::new(&connection)
        .interface("org.kde.ScreenSaver")?
        .path("/ScreenSaver")?
        .destination("org.kde.ScreenSaver")?
        .build()?;

    let response = proxy.call_method("GetSessionIdleTime", &())?;
    let idle_ms: u64 = response.body().deserialize()?;
    Ok(idle_ms / 1000)
}

/// Wayland idle through systemd-logind. `IdleSinceHintMonotonic` is a monotonic
/// timestamp in microseconds, so unlike `hyprctl getoption idle_inactivity` it
/// represents elapsed inactivity rather than a configured timeout. Hyprland
/// does not expose a portable compositor-wide idle query, so we must not treat
/// its configured `idle_inactivity` value as the user's current idle duration.
#[cfg(target_os = "linux")]
fn get_idle_via_logind() -> Result<u64, Box<dyn std::error::Error>> {
    let session = std::env::var("XDG_SESSION_ID")?;
    let output = Command::new("loginctl")
        .args(["show-session", &session, "--property=IdleHint", "--property=IdleSinceHintMonotonic"])
        .output()?;

    if !output.status.success() {
        return Err("loginctl show-session failed".into());
    }
    parse_logind_idle_seconds(&String::from_utf8_lossy(&output.stdout))
}

#[cfg(target_os = "linux")]
fn parse_logind_idle_seconds(properties: &str) -> Result<u64, Box<dyn std::error::Error>> {
    let mut idle_hint = false;
    let mut since_monotonic_us = None;

    for line in properties.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match key {
            "IdleHint" => idle_hint = value == "yes",
            "IdleSinceHintMonotonic" => since_monotonic_us = Some(value.parse::<u64>()?),
            _ => {}
        }
    }

    if !idle_hint {
        return Ok(0);
    }

    let since = since_monotonic_us.ok_or("logind did not provide idle timestamp")?;
    let uptime = std::fs::read_to_string("/proc/uptime")?
        .split_whitespace()
        .next()
        .ok_or("/proc/uptime was empty")?
        .parse::<f64>()?;
    let now_us = (uptime * 1_000_000.0) as u64;
    Ok(now_us.saturating_sub(since) / 1_000_000)
}

/// X11 Screen Saver extension (XSS). `XScreenSaverQueryInfo.idle` is ms.
#[cfg(target_os = "linux")]
fn get_idle_via_x11() -> Result<u64, Box<dyn std::error::Error>> {
    use x11_dl::{xlib, xss};

    let xlib = xlib::Xlib::open().map_err(|_| "Could not load X11 library")?;
    let xss = xss::Xss::open().map_err(|_| "Could not load Xss library")?;

    let display = unsafe { (xlib.XOpenDisplay)(ptr::null()) };
    if display.is_null() {
        return Err("Could not open X display".into());
    }

    let res = unsafe {
        let root = (xlib.XDefaultRootWindow)(display);
        let mut info = std::mem::MaybeUninit::<xss::XScreenSaverInfo>::uninit();
        let ret = (xss.XScreenSaverQueryInfo)(display, root, info.as_mut_ptr());
        (xlib.XCloseDisplay)(display);
        if ret != 0 {
            Ok(info.assume_init().idle as u64 / 1000)
        } else {
            Err("XScreenSaverQueryInfo failed".into())
        }
    };
    res
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::parse_logind_idle_seconds;

    #[test]
    fn active_session_reports_zero_idle_seconds() {
        let properties = "IdleHint=no\nIdleSinceHintMonotonic=123456\n";
        assert_eq!(parse_logind_idle_seconds(properties).unwrap(), 0);
    }

    #[test]
    fn malformed_idle_timestamp_is_rejected() {
        let properties = "IdleHint=yes\nIdleSinceHintMonotonic=not-a-number\n";
        assert!(parse_logind_idle_seconds(properties).is_err());
    }
}