//! Platform process observation behind one trait.
//!
//! - Linux reads `/proc` directly: a pid is user activity when its `exe`
//!   link resolves (kernel threads and zombies have no `exe`). The raw
//!   material (pid, resolved executable path, command line) stays inside the
//!   core; only the engine's sanitized records ever leave it.
//! - Windows enumerates image names via the inbox `tasklist` command (no new
//!   dependencies): each row yields a pid plus an exe basename, which is
//!   enough for the engine's label/app-id/icon pipeline.
//!
//! Platforms without an implementation report `unsupported` — the activity
//! page then shows honest emptiness, never invented moments.

use std::io;
use std::path::PathBuf;

use super::activity::RawObservation;

/// One scan of the running system.
pub trait ProcessMonitor: Send {
    fn scan(&mut self) -> io::Result<Vec<RawObservation>>;
}

/// Reads `/proc/<pid>/exe` and `/proc/<pid>/cmdline`.
pub struct LinuxProcessMonitor;

impl ProcessMonitor for LinuxProcessMonitor {
    fn scan(&mut self) -> io::Result<Vec<RawObservation>> {
        let mut observations = Vec::new();
        let entries = std::fs::read_dir("/proc")?;
        for entry in entries {
            let Ok(entry) = entry else { continue };
            let Some(file_name) = entry.file_name().to_str().map(String::from) else {
                continue;
            };
            let Ok(pid) = file_name.parse::<u32>() else {
                continue;
            };
            let process_dir = PathBuf::from("/proc").join(file_name);

            // Kernel threads and zombies have no resolvable exe link.
            let Ok(exe_path) = std::fs::read_link(process_dir.join("exe")) else {
                continue;
            };
            let command_line = std::fs::read_to_string(process_dir.join("cmdline"))
                .ok()
                .map(|line| line.trim_end_matches('\0').to_string());

            observations.push(RawObservation {
                pid,
                exe_path: Some(exe_path.to_string_lossy().into_owned()),
                command_line,
            });
        }
        Ok(observations)
    }
}

/// Placeholder for platforms without an implementation yet: scanning always
/// reports unsupported so the UI can show real unavailability.
pub struct UnsupportedProcessMonitor;

impl ProcessMonitor for UnsupportedProcessMonitor {
    fn scan(&mut self) -> io::Result<Vec<RawObservation>> {
        // A typed Unsupported kind lets the engine distinguish "this
        // platform has no monitor" from a transient scan failure.
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }
}

/// Windows monitor without new dependencies: parses the inbox `tasklist`
/// CSV (`"Image Name","PID",…`). Image names are basenames (`notepad.exe`);
/// the engine derives label/app-id/icon from them exactly like a `/proc`
/// exe path basename.
#[cfg(target_os = "windows")]
pub struct WindowsProcessMonitor;

#[cfg(target_os = "windows")]
impl ProcessMonitor for WindowsProcessMonitor {
    fn scan(&mut self) -> io::Result<Vec<RawObservation>> {
        let output = std::process::Command::new("tasklist")
            .args(["/FO", "CSV", "/NH"])
            .output()
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        if !output.status.success() {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                "tasklist exited non-zero",
            ));
        }
        Ok(parse_tasklist_csv(&String::from_utf8_lossy(&output.stdout)))
    }
}

#[cfg(target_os = "windows")]
#[allow(dead_code)]
fn parse_tasklist_csv(text: &str) -> Vec<RawObservation> {
    parse_tasklist_csv_inner(text)
}

/// Pure CSV parser kept platform-independent so unit tests exercise it on
/// Linux too; the Windows monitor is the only production caller.
fn parse_tasklist_csv_inner(text: &str) -> Vec<RawObservation> {
    let mut out = Vec::new();
    for line in text.lines() {
        // `"firefox.exe","1234","Console","1","200,000 K"`
        let fields = split_csv_line(line);
        if fields.len() < 2 {
            continue;
        }
        let image = fields[0].trim().trim_matches('"');
        let pid_text = fields[1].trim().trim_matches('"');
        let Ok(pid) = pid_text.parse::<u32>() else {
            continue;
        };
        if pid == 0 || image.is_empty() || image.eq_ignore_ascii_case("System Idle Process") {
            continue;
        }
        // Keep only plausible image names; the engine sanitizes the rest.
        if image.len() > 260 || image.contains(['/', '\\', ':']) {
            continue;
        }
        out.push(RawObservation {
            pid,
            exe_path: Some(image.to_string()),
            command_line: None,
        });
    }
    out
}

/// Quote-aware CSV split for `tasklist` rows.
fn split_csv_line(line: &str) -> Vec<&str> {
    // tasklist quotes every field and never embeds escaped quotes in the
    // first two columns; a quote-aware split is enough.
    let mut fields = Vec::new();
    let mut start = 0;
    let mut in_quotes = false;
    for (i, c) in line.char_indices() {
        if c == '"' {
            in_quotes = !in_quotes;
        } else if c == ',' && !in_quotes {
            fields.push(line[start..i].trim());
            start = i + 1;
        }
    }
    fields.push(line[start..].trim());
    fields
}

/// The monitor for the running platform.
pub fn platform_monitor() -> Box<dyn ProcessMonitor> {
    if cfg!(target_os = "linux") {
        Box::new(LinuxProcessMonitor)
    } else if cfg!(target_os = "windows") {
        #[cfg(target_os = "windows")]
        {
            return Box::new(WindowsProcessMonitor);
        }
        #[cfg(not(target_os = "windows"))]
        {
            return Box::new(UnsupportedProcessMonitor);
        }
    } else {
        Box::new(UnsupportedProcessMonitor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linux_monitor_finds_at_least_the_current_process() {
        if !cfg!(target_os = "linux") {
            return;
        }
        let mut monitor = LinuxProcessMonitor;
        let observations = monitor.scan().unwrap();
        assert!(
            observations.iter().any(|o| o.pid == std::process::id()),
            "the scanning process itself must be visible"
        );
        // Raw material is present locally (that is the point of the local
        // record), and the test process has a real exe.
        let self_observation = observations
            .iter()
            .find(|o| o.pid == std::process::id())
            .unwrap();
        assert!(self_observation.exe_path.is_some());
    }

    #[test]
    fn tasklist_csv_parses_image_names_and_pids() {
        let sample = "\"firefox.exe\",\"1234\",\"Console\",\"1\",\"200,000 K\"\r\n\
            \"Code.exe\",\"5678\",\"Console\",\"1\",\"150,000 K\"\r\n\
            \"System Idle Process\",\"0\",\"Services\",\"0\",\"8 K\"\r\n\
            \"badline without pid\"\r\n";
        let observations = parse_tasklist_csv_inner(sample);
        assert_eq!(observations.len(), 2);
        assert_eq!(observations[0].pid, 1234);
        assert_eq!(observations[0].exe_path.as_deref(), Some("firefox.exe"));
        assert_eq!(observations[1].pid, 5678);
        assert_eq!(observations[1].exe_path.as_deref(), Some("Code.exe"));
    }
}
