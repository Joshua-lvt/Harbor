//! Real app-icon extraction for Linux (Feature 4).
//!
//! `get_app_icon(exe_path)` extracts the application's icon, resizes it to
//! 48×48, encodes it as a PNG, and returns it as a base64 `data:image/png` URL
//! so the frontend can cache + send it once per exe over the WS
//! (`activity_icon`). On Linux we look the icon up by the executable's
//! basename in the freedesktop icon-theme search path, falling back to the
//! `Icon=` field of matching `.desktop` files. On any failure we return
//! `None` — the frontend then shows a `GeneratedAppIcon` fallback.
//!
//! Privacy: we only ever read the icon of an executable that the user's OWN
//! machine is running — never the window title or any screen content. The
//! icon crosses the wire once per new exe so the partner renders the real
//! program icon.

use serde::Serialize;
use base64::Engine;
use std::path::Path;
use std::process::Command;

/// The foreground app, returned to the frontend. `exe` is the lowercased
/// basename (e.g. "code") used as the activity key + icon cache key; `path`
/// is the full image-name (e.g. "/usr/bin/code") used here to extract the
/// once-per-exe icon. Either may be `None` on failure.
#[derive(Serialize)]
pub struct ForegroundApp {
    pub exe: Option<String>,
    pub path: Option<String>,
}

/// Extract the icon for an exe PATH, returning a base64 `data:image/png` URL
/// (48×48). `None` on extraction failure (→ generated fallback).
#[tauri::command]
pub fn get_app_icon(exe_path: String) -> Option<String> {
    let _ = exe_path;
    get_app_icon_linux(&exe_path)
}

/// Linux icon extraction. Tries multiple strategies in order of preference:
///  1. `xdg-icon-resource lookup` (the freedesktop-standard lookup).
///  2. Icon-theme search path by the executable basename (hicolor + themed).
///  3. Parse matching `.desktop` files for an `Icon=` field, then resolve it
///     through the theme search path.
fn get_app_icon_linux(exe_path: &str) -> Option<String> {
    let app_name = Path::new(exe_path)
        .file_name()
        .and_then(|s| s.to_str())?
        // Strip a trailing ".exe" so accidental Windows-style names still match.
        .trim_end_matches(".exe")
        .to_string();

    // 1. xdg-icon-resource lookup.
    if let Some(icon) = lookup_icon_xdg(&app_name) {
        return Some(icon);
    }

    // 2. Direct theme search by app name.
    let data_dirs = icon_search_dirs();
    if let Some(icon) = find_icon_in_dirs(&data_dirs, &app_name) {
        return Some(icon);
    }

    // 3. .desktop file Icon= field, resolved through the theme search path.
    if let Some(icon_name) = icon_name_from_desktop(&app_name) {
        if let Some(icon) = find_icon_in_dirs(&data_dirs, &icon_name) {
            return Some(icon);
        }
    }

    None
}

/// Run `xdg-icon-resource lookup [--size 48] <name>` and, if it prints a real
/// path, load + re-encode it. Some distros ship a `lookup` subcommand; others
/// don't — both outcomes are handled (the dir-walk below covers the gap).
fn lookup_icon_xdg(app_name: &str) -> Option<String> {
    let candidates = [
        // 48×48 (our target size) first.
        vec!["lookup", "--size", "48", app_name],
        // Any size.
        vec!["lookup", app_name],
    ];
    for args in candidates {
        let output = Command::new("xdg-icon-resource").args(&args).output();
        if let Ok(out) = output {
            if out.status.success() {
                let path = String::from_utf8(out.stdout).ok()?.trim().to_string();
                if !path.is_empty() && Path::new(&path).exists() {
                    if let Some(data_url) = load_and_encode_icon(&path) {
                        return Some(data_url);
                    }
                }
            }
        }
    }
    None
}

/// The freedesktop icon-theme search path: XDG_DATA_DIRS/icons, plus the
/// common system locations. The `dirs` crate resolves `data_dir()` per user,
/// but we also walk the full XDG_DATA_DIRS so flatpak/snap and custom installs
/// are covered.
fn icon_search_dirs() -> Vec<std::path::PathBuf> {
    let mut dirs: Vec<std::path::PathBuf> = Vec::new();

    // Per-user data dir (~/.local/share).
    if let Some(d) = dirs_next::data_dir() {
        dirs.push(d.join("icons"));
    }
    // XDG_DATA_DIRS (colon-separated) — the freedesktop-specified search path.
    if let Ok(xdg) = std::env::var("XDG_DATA_DIRS") {
        for entry in xdg.split(':') {
            if entry.is_empty() {
                continue;
            }
            dirs.push(Path::new(entry).join("icons"));
        }
    } else {
        // Default per spec when XDG_DATA_DIRS is unset.
        dirs.push(Path::new("/usr/local/share/icons").to_path_buf());
        dirs.push(Path::new("/usr/share/icons").to_path_buf());
    }
    // hicolor is the guaranteed fallback theme.
    dirs.push(Path::new("/usr/share/icons/hicolor").to_path_buf());
    dirs
}

/// Walk the icon-theme search dirs for `<name>` at the preferred sizes +
/// categories, returning the first on-disk icon path found (PNG/SVG/XPM).
fn find_icon_in_dirs(
    search_dirs: &[std::path::PathBuf],
    icon_name: &str,
) -> Option<String> {
    // Preferred sizes (our target 48 first) and categories per the spec.
    let sizes = ["48x48", "32x32", "64x64", "24x24", "96x96", "128x128", "256x256", "scalable", "512x512"];
    let categories = ["apps", "applications", "devices", "mimetypes"];
    let extensions = ["png", "svg", "xpm"];

    for base in search_dirs {
        for size in sizes {
            for category in categories {
                for ext in extensions {
                    let p = base.join(size).join(category).join(format!("{}.{}", icon_name, ext));
                    if p.exists() {
                        if let Some(url) = load_and_encode_icon(&p.to_string_lossy()) {
                            return Some(url);
                        }
                    }
                }
            }
        }
    }

    // Last resort: an unconditional recursive walk of the search dirs. Covers
    // themes with non-standard layouts (e.g. flatpak exports).
    for base in search_dirs {
        if !base.exists() {
            continue;
        }
        if let Some(found) = find_icon_recursive(base, icon_name) {
            if let Some(url) = load_and_encode_icon(&found) {
                return Some(url);
            }
        }
    }
    None
}

/// Recursive walk for an icon file matching `name` (file stem) anywhere under
/// `dir`. Returns the first matching path. Bounded by the caller's search dirs.
fn find_icon_recursive(dir: &Path, icon_name: &str) -> Option<String> {
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = find_icon_recursive(&path, icon_name) {
                return Some(found);
            }
        } else if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
            if stem == icon_name
                && path
                    .extension()
                    .and_then(|s| s.to_str())
                    .map(|e| matches!(e.to_lowercase().as_str(), "png" | "svg" | "xpm"))
                    .unwrap_or(false)
            {
                return Some(path.to_string_lossy().to_string());
            }
        }
    }
    None
}

/// Scan the freedesktop `.desktop` search path for an entry whose `Exec=` line
/// references `app_name`, and return its `Icon=` field value (the icon NAME,
/// not a path — it still needs theme resolution).
fn icon_name_from_desktop(app_name: &str) -> Option<String> {
    let mut desktop_dirs: Vec<std::path::PathBuf> = Vec::new();
    if let Some(d) = dirs_next::data_dir() {
        desktop_dirs.push(d.join("applications"));
    }
    for base in ["/usr/local/share/applications", "/usr/share/applications"] {
        desktop_dirs.push(Path::new(base).to_path_buf());
    }

    for dir in desktop_dirs {
        if !dir.exists() {
            continue;
        }
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|s| s.to_str()) != Some("desktop") {
                    continue;
                }
                let content = match std::fs::read_to_string(&path) {
                    Ok(c) => c,
                    Err(_) => continue,
                };
                // Match the executable: `Exec=<name>` or `Exec=<name> ...`.
                // Also accept `Exec=.*<name>` for wrapper scripts.
                let matches_exec = content.lines().any(|line| {
                    if let Some(rest) = line.strip_prefix("Exec=") {
                        let first = rest.split_whitespace().next().unwrap_or("");
                        // Compare basenames so /usr/bin/foo matches Exec=/usr/bin/foo.
                        let exec_base = Path::new(first)
                            .file_name()
                            .and_then(|s| s.to_str())
                            .unwrap_or(first);
                        exec_base == app_name
                    } else {
                        false
                    }
                });
                if !matches_exec {
                    continue;
                }
                for line in content.lines() {
                    if let Some(icon_name) = line.strip_prefix("Icon=") {
                        let icon_name = icon_name.trim();
                        if !icon_name.is_empty() && Path::new(icon_name).is_absolute() {
                            // Absolute path Icon= — return as-is so the caller
                            // can load it directly.
                            return Some(icon_name.to_string());
                        } else if !icon_name.is_empty() {
                            return Some(icon_name.to_string());
                        }
                    }
                }
            }
        }
    }
    None
}

/// Load an image file from disk, resize it to 48×48, encode as PNG, and return
/// a base64 `data:image/png` URL. Returns `None` on any decode/encode failure
/// (the caller falls back to the generated icon).
fn load_and_encode_icon(icon_path: &str) -> Option<String> {
    let image_data = std::fs::read(icon_path).ok()?;

    // SVG needs the `image` crate's svg feature; PNG/XPM decode from memory.
    // Decode, downcast to RGBA8, resize with NEAREST (crisp for icons).
    let img = image::load_from_memory(&image_data).ok()?;
    let rgba = img.to_rgba8();

    let small = image::imageops::resize(&rgba, 48, 48, image::imageops::FilterType::Nearest);

    let mut png_data = Vec::new();
    small
        .write_to(&mut std::io::Cursor::new(&mut png_data), image::ImageFormat::Png)
        .ok()?;

    let b64 = base64::engine::general_purpose::STANDARD.encode(&png_data);
    Some(format!("data:image/png;base64,{}", b64))
}

// A tiny inline re-export of the `dirs`-style helpers so we don't add a second
// dependency (Cargo.toml already pins `dirs = "5.0"`). Used above for per-user
// data dir + XDG_DATA_DIRS resolution.
mod dirs_next {
    use std::path::PathBuf;

    pub fn data_dir() -> Option<PathBuf> {
        ::dirs::data_dir()
    }
}