//! Cross-platform app identity and icon keys without new dependencies.
//!
//! The activity engine only ever saw a sanitized label (`firefox`,
//! `Code`). The UI needs a stable `app_id` plus a theme-safe `icon_key`
//! to render the program's real icon through the OS layer:
//! - Linux: freedesktop `.desktop` `Icon=` names resolved by
//!   `QIcon::fromTheme` in the Qt layer.
//! - Windows: the exe basename resolved by `QFileIconProvider` /
//!   `findExecutable` in the Qt layer.
//!
//! Absolute icon paths never leave the device: this module only emits
//! theme-safe keys (`firefox`, `code`, `vlc`). No new crates, `std` only.

/// Maximum length of an app id or icon key. Matches the activity label
/// budget so keys stay safe for IPC, QML, and peer records.
pub const APP_ID_MAX: usize = 64;

/// Normalized, shareable app identity from an executable path.
///
/// Handles both `/` (Linux) and `\` (Windows) separators plus the
/// `\\?\` extended-path prefix. Strips `.exe`/`.bin` case-insensitively,
/// lowercases, and keeps only theme-safe characters.
pub fn normalize_app_id(exe_path: &str) -> Option<String> {
    let base = basename_os(exe_path)?;
    let mut name = base.to_string();
    // Strip a single executable suffix, case-insensitively.
    let lowered_suffix = name.to_lowercase();
    for suffix in [".exe", ".bin", ".appimage"] {
        if lowered_suffix.ends_with(suffix) {
            name = name[..name.len() - suffix.len()].to_string();
            break;
        }
    }
    let lowered = name.to_lowercase();
    let mut out = String::with_capacity(lowered.len());
    let mut last_dash = false;
    for c in lowered.chars() {
        if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '+') {
            out.push(c);
            last_dash = false;
        } else if c == ' ' || c == '(' || c == ')' || c == '[' || c == ']' {
            if !out.is_empty() && !last_dash {
                out.push('-');
                last_dash = true;
            }
        } else if !out.is_empty() && !last_dash {
            // Separators inside versioned names (`foo-1.2`) stay readable;
            // anything else becomes a single dash.
            out.push('-');
            last_dash = true;
        }
    }
    let trimmed = out.trim_matches(|c| c == '-' || c == '.').to_string();
    if trimmed.is_empty() || trimmed.chars().all(|c| c == '.') {
        return None;
    }
    let truncated: String = trimmed.chars().take(APP_ID_MAX).collect();
    let truncated = truncated.trim_matches(|c| c == '-' || c == '.').to_string();
    if truncated.is_empty() {
        None
    } else {
        Some(truncated)
    }
}

/// Theme-safe icon key for an executable path.
///
/// - Linux: looks up the freedesktop `.desktop` entry whose `Exec=`
///   basename matches the app, and returns its sanitized `Icon=` theme
///   name. Falls back to the normalized app id (which `QIcon::fromTheme`
///   often resolves directly, e.g. `firefox`).
/// - Other platforms: returns the normalized app id so the Qt layer can
///   resolve it natively (`QFileIconProvider` on Windows).
pub fn resolve_icon_key(exe_path: &str) -> Option<String> {
    let app_id = normalize_app_id(exe_path)?;
    if cfg!(target_os = "linux") {
        if let Some(icon) = find_desktop_icon(&app_id) {
            return Some(icon);
        }
    }
    Some(app_id)
}

/// Sanitizes a raw `Icon=` value (or theme name) into a shareable key.
///
/// Absolute paths are reduced to their file stem (`/…/firefox.png` →
/// `firefox`) so no local path ever leaves the device. Returns `None`
/// when nothing theme-safe remains.
pub fn sanitize_icon_key(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    // A relative traversal is not an icon identity.  Do not reduce it to a
    // misleading basename (for example, `../../etc/passwd` -> `passwd`).
    if trimmed
        .split(['/', '\\'])
        .any(|component| component == "." || component == "..")
    {
        return None;
    }
    let path_like = trimmed.contains('/') || trimmed.contains('\\');
    // Absolute or themed path: keep only the file stem.
    let stem_source = if path_like {
        let base = basename_os(trimmed).unwrap_or(trimmed);
        // Strip well-known raster/vector suffixes case-insensitively.
        let lower = base.to_lowercase();
        let mut stem = base;
        for suffix in [".png", ".svg", ".xpm", ".ico"] {
            if lower.ends_with(suffix) {
                stem = &base[..base.len() - suffix.len()];
                break;
            }
        }
        stem
    } else {
        trimmed
    };
    let cleaned: String = stem_source
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '+'))
        .collect::<String>();
    // Windows executable/icon paths are case-insensitive.  Normalize only
    // path-derived keys; freedesktop theme names without a path retain their
    // spelling because those names can be case-sensitive.
    let cleaned = if path_like {
        cleaned.to_lowercase()
    } else {
        cleaned
    };
    let cleaned = cleaned.trim_matches('.').to_string();
    if cleaned.is_empty() || cleaned.len() > APP_ID_MAX {
        // Over-long vendor names are not theme keys; fall back to the
        // caller using the app id instead.
        if cleaned.len() > APP_ID_MAX {
            return None;
        }
        return None;
    }
    if cleaned.chars().all(|c| c == '.') {
        return None;
    }
    Some(cleaned)
}

/// Last path component handling both `/` and `\`, plus the Windows
/// `\\?\` extended-path prefix.
fn basename_os(path: &str) -> Option<&str> {
    let mut trimmed = path.trim();
    if trimmed.is_empty() {
        return None;
    }
    // Strip Windows extended-path / device prefixes: `\\?\C:\…`, `\\.\…`.
    for prefix in ["\\\\?\\", "\\\\.\\"] {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            trimmed = rest;
            break;
        }
    }
    // Trailing separators carry no name.
    let trimmed = trimmed.trim_end_matches(['/', '\\']);
    if trimmed.is_empty() {
        return None;
    }
    let after_slash = trimmed.rsplit('/').next().unwrap_or(trimmed);
    let base = after_slash.rsplit('\\').next().unwrap_or(after_slash);
    // Windows drive roots (`C:`) are not app names.
    let base = base.trim();
    if base.is_empty()
        || base == "."
        || base == ".."
        || (base.len() == 2
            && base.as_bytes()[1] == b':'
            && base.as_bytes()[0].is_ascii_alphabetic())
    {
        return None;
    }
    Some(base)
}

/// Directories that may contain an `applications` folder with `.desktop`
/// files, highest priority first.
fn desktop_base_dirs() -> Vec<std::path::PathBuf> {
    use std::path::PathBuf;
    let mut dirs = Vec::new();
    if let Some(home_data) = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from) {
        dirs.push(home_data);
    }
    if let Ok(data_dirs) = std::env::var("XDG_DATA_DIRS") {
        for part in data_dirs.split(':').filter(|p| !p.is_empty()) {
            dirs.push(PathBuf::from(part));
        }
    }
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        dirs.push(home.join(".local/share"));
    }
    for fallback in [
        "/usr/share",
        "/usr/local/share",
        "/var/lib/flatpak/exports/share",
        "/var/lib/snapd/desktop",
        "/snap/share",
    ] {
        dirs.push(PathBuf::from(fallback));
    }
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        dirs.push(home.join(".local/share/flatpak/exports/share"));
    }
    // Deduplicate while keeping order.
    let mut seen = std::collections::HashSet::new();
    dirs.into_iter()
        .filter(|d| seen.insert(d.clone()))
        .collect()
}

/// Searches `.desktop` files for one whose `Exec=` basename matches
/// `app_id` (case-insensitive), returning its sanitized `Icon=` key.
fn find_desktop_icon(app_id: &str) -> Option<String> {
    let wanted = app_id.to_lowercase();
    for base in desktop_base_dirs() {
        let apps = base.join("applications");
        let entries = std::fs::read_dir(&apps).ok()?;
        // Hundreds of desktop files exist on a full system; cap the scan
        // so a burst of new processes cannot stall the monitor tick.
        for entry in entries.take(400).flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("desktop") {
                continue;
            }
            if let Some(icon) = desktop_entry_icon(&path, &wanted) {
                return sanitize_icon_key(&icon).or_else(|| Some(app_id.to_string()));
            }
        }
    }
    None
}

/// Parses one `.desktop` file (bounded read): returns the `Icon=` value
/// when its `Exec=` basename matches `wanted` (already lowercased).
fn desktop_entry_icon(path: &std::path::Path, wanted: &str) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    if bytes.len() > 32 * 1024 {
        return None;
    }
    let text = String::from_utf8_lossy(&bytes);
    let mut in_entry = false;
    let mut exec_match = false;
    let mut icon: Option<String> = None;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
            if !in_entry && exec_match {
                break;
            }
            continue;
        }
        if !in_entry {
            continue;
        }
        if let Some(exec) = line.strip_prefix("Exec=") {
            // First token is the binary; strip quotes, env wrappers, args.
            let first = exec.trim().trim_matches('"').split_whitespace().next()?;
            let first = first.trim_matches('"').trim_matches('\'');
            // `env FOO=bar /usr/bin/app` wrappers: take the last token that
            // looks like a path/binary rather than an assignment.
            let mut candidate = first;
            for token in exec.split_whitespace().rev() {
                let t = token.trim_matches('"').trim_matches('\'');
                if t.is_empty() || t.contains('=') || t.starts_with('%') || t.starts_with('-') {
                    continue;
                }
                candidate = t;
                break;
            }
            let base = basename_os(candidate).unwrap_or(candidate);
            // Compare with and without extension (`code` vs `code.exe`).
            let base_lower = base.to_lowercase();
            let base_stripped = base_lower
                .strip_suffix(".exe")
                .or_else(|| base_lower.strip_suffix(".bin"))
                .unwrap_or(&base_lower);
            if base_lower == wanted || base_stripped == wanted {
                exec_match = true;
            }
        } else if let Some(value) = line.strip_prefix("Icon=") {
            icon = Some(value.trim().to_string());
        }
        if exec_match && icon.is_some() {
            break;
        }
    }
    if exec_match { icon } else { None }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_linux_and_windows_paths() {
        assert_eq!(
            normalize_app_id("/usr/bin/firefox"),
            Some("firefox".to_string())
        );
        assert_eq!(
            normalize_app_id("C:\\Program Files\\Mozilla Firefox\\firefox.exe"),
            Some("firefox".to_string())
        );
        assert_eq!(
            normalize_app_id("C:/Program Files/VS Code/Code.exe"),
            Some("code".to_string())
        );
        assert_eq!(
            normalize_app_id("\\\\?\\C:\\Windows\\System32\\notepad.exe"),
            Some("notepad".to_string())
        );
        assert_eq!(normalize_app_id("/opt/Discord/Discord"), Some("discord".to_string()));
        assert_eq!(normalize_app_id("C:"), None);
        assert_eq!(normalize_app_id(""), None);
    }

    #[test]
    fn sanitizes_icon_values_without_leaking_paths() {
        assert_eq!(
            sanitize_icon_key("firefox"),
            Some("firefox".to_string())
        );
        assert_eq!(
            sanitize_icon_key("/usr/share/icons/hicolor/48x48/apps/firefox.png"),
            Some("firefox".to_string())
        );
        assert_eq!(
            sanitize_icon_key("C:\\Icons\\Code.ico"),
            Some("code".to_string())
        );
        assert_eq!(sanitize_icon_key("../../etc/passwd"), None);
        assert_eq!(sanitize_icon_key(""), None);
    }

    #[test]
    fn finds_desktop_icon_in_injected_dir() {
        let dir = std::env::temp_dir().join(format!("harbor-icon-test-{}", uuid_stamp()));
        let apps = dir.join("applications");
        std::fs::create_dir_all(&apps).unwrap();
        std::fs::write(
            apps.join("myapp.desktop"),
            "[Desktop Entry]\nName=My App\nExec=/usr/bin/myapp %U\nIcon=myapp-theme\n",
        )
        .unwrap();
        // Temporarily point XDG_DATA_HOME at the fixture.
        let old = std::env::var_os("XDG_DATA_HOME");
        unsafe { std::env::set_var("XDG_DATA_HOME", &dir) };
        let found = find_desktop_icon("myapp");
        match old {
            Some(v) => unsafe { std::env::set_var("XDG_DATA_HOME", v) },
            None => unsafe { std::env::remove_var("XDG_DATA_HOME") },
        }
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(found, Some("myapp-theme".to_string()));
    }

    fn uuid_stamp() -> String {
        use std::time::{SystemTime, UNIX_EPOCH};
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        format!("{}-{}", std::process::id(), nanos)
    }
}
