//! Supervision boundary for Harbor's private Pion worker.
//!
//! This module intentionally does not know a Harbor identity, a peer, or the
//! control-plane server. It owns only the bounded framed stdio channel to a
//! child process started by the Rust core.

use std::collections::BTreeSet;
use std::env;
use std::io::{self, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command as ProcessCommand, Stdio};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use uuid::Uuid;

const MEDIA_PROTOCOL_VERSION: u16 = 1;
const MEDIA_MAX_FRAME_BYTES: usize = 256 * 1024;
const COMMAND_QUEUE_CAPACITY: usize = 8;
const EVENT_QUEUE_CAPACITY: usize = 64;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(3);
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, Deserialize, Serialize)]
struct MediaEnvelope {
    #[serde(rename = "v")]
    version: u16,
    #[serde(rename = "type")]
    message_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    request_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    event_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reply_to: Option<String>,
    timestamp: String,
    payload: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    error: Option<MediaProtocolError>,
}

impl MediaEnvelope {
    fn request(message_type: &str, payload: Value) -> Self {
        Self {
            version: MEDIA_PROTOCOL_VERSION,
            message_type: message_type.into(),
            request_id: Some(Uuid::new_v4().to_string()),
            event_id: None,
            reply_to: None,
            timestamp: crate::rfc3339_now(),
            payload,
            error: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MediaProtocolError {
    pub code: String,
    pub ui_key: String,
    pub retryable: bool,
    detail: String,
}

#[derive(Debug, Clone)]
pub struct MediaReply {
    pub message_type: String,
    pub payload: Value,
    pub error: Option<MediaProtocolError>,
    /// The in-flight request this reply correlates with; validated by the
    /// command loop before it reaches a caller.
    pub reply_to: String,
}

#[derive(Debug, Clone)]
pub struct MediaEvent {
    pub message_type: String,
    pub payload: Value,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum MediaError {
    #[error("media worker executable is unavailable")]
    ExecutableUnavailable,
    #[error("media worker standard I/O is unavailable")]
    MissingPipe,
    #[error("media worker has exited")]
    Exited,
    #[error("media worker command queue is full")]
    Busy,
    #[error("media worker did not answer before its deadline")]
    TimedOut,
    #[error("media worker returned an invalid private envelope")]
    InvalidEnvelope,
    #[error("media worker returned an unexpected reply")]
    UnexpectedReply,
    #[error("media worker I/O failed")]
    Io,
}

struct MediaCommand {
    request: MediaEnvelope,
    reply: mpsc::SyncSender<Result<MediaReply, MediaError>>,
}

/// A single supervised Pion child. A reader thread owns stdout continuously
/// (so asynchronous facts are never stranded in the pipe), while a serialized
/// command loop owns stdin and correlates replies. The outer core only
/// receives bounded facts.
pub struct MediaSupervisor {
    child: Arc<Mutex<Child>>,
    commands: mpsc::SyncSender<MediaCommand>,
    events: mpsc::Receiver<MediaEvent>,
}

impl MediaSupervisor {
    /// Starts the worker. `notify` receives one token whenever worker facts
    /// have been queued (or the worker has died), so the owning core can drain
    /// them without waiting for its next dispatch.
    pub fn start(notify: Option<mpsc::Sender<()>>) -> Result<Self, MediaError> {
        Self::start_with_executable(None, notify)
    }

    /// Starts the worker at an explicit path supplied by the host. Android
    /// extracts private executables from the APK into its app data directory;
    /// the core must not guess that path from `current_exe()` (which points at
    /// the Qt loader, not an APK asset). `None` retains the desktop/env
    /// resolution used by the existing stdio application and tests.
    pub fn start_with_executable(
        explicit: Option<&Path>,
        notify: Option<mpsc::Sender<()>>,
    ) -> Result<Self, MediaError> {
        let executable = match explicit {
            Some(path) if !path.as_os_str().is_empty() => path.to_path_buf(),
            _ => resolve_worker_executable()?,
        };
        let mut process = ProcessCommand::new(executable)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // The worker's private diagnostic stream must never block it or
            // cross the typed UI boundary.
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| MediaError::ExecutableUnavailable)?;
        let stdin = process.stdin.take().ok_or(MediaError::MissingPipe)?;
        let stdout = process.stdout.take().ok_or(MediaError::MissingPipe)?;
        let child = Arc::new(Mutex::new(process));
        let (commands, command_receiver) = mpsc::sync_channel(COMMAND_QUEUE_CAPACITY);
        let (event_sender, events) = mpsc::sync_channel(EVENT_QUEUE_CAPACITY);

        let worker_child = Arc::clone(&child);
        thread::spawn(move || {
            run_worker(
                stdin,
                stdout,
                worker_child,
                command_receiver,
                event_sender,
                notify,
            )
        });

        let supervisor = Self {
            child,
            commands,
            events,
        };
        let hello = supervisor.request("media.hello", json!({}))?;
        let service = hello.payload.get("service").and_then(Value::as_str);
        let protocol = hello.payload.get("protocol").and_then(Value::as_u64);
        let capabilities = hello
            .payload
            .get("capabilities")
            .and_then(Value::as_array)
            .map(|values| {
                values
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<BTreeSet<_>>()
            });
        if hello.message_type != "media.hello"
            || hello.error.is_some()
            || service != Some("harbor-media")
            || protocol != Some(MEDIA_PROTOCOL_VERSION.into())
            || capabilities
                .as_ref()
                .is_none_or(|items| !items.contains("host-ice") || !items.contains("local-offer"))
        {
            supervisor.kill();
            return Err(MediaError::InvalidEnvelope);
        }
        Ok(supervisor)
    }

    /// Sends a single command and waits for its correlated response. The
    /// deadline is enforced inside the IO loop, which kills the child rather
    /// than letting a wedged private worker survive the core.
    pub fn request(&self, message_type: &str, payload: Value) -> Result<MediaReply, MediaError> {
        if !self.is_running() {
            return Err(MediaError::Exited);
        }
        let request = MediaEnvelope::request(message_type, payload);
        let (reply_sender, reply_receiver) = mpsc::sync_channel(1);
        self.commands
            .try_send(MediaCommand {
                request,
                reply: reply_sender,
            })
            .map_err(|error| match error {
                mpsc::TrySendError::Full(_) => MediaError::Busy,
                mpsc::TrySendError::Disconnected(_) => MediaError::Exited,
            })?;
        match reply_receiver.recv() {
            Ok(result) => result,
            Err(_) => Err(MediaError::Exited),
        }
    }

    /// Returns worker events that were received while a command was in flight.
    /// The core decides which facts become local UI events; SDP/ICE never cross
    /// this boundary directly into QML.
    pub fn drain_events(&self) -> Vec<MediaEvent> {
        let mut events = Vec::new();
        while let Ok(event) = self.events.try_recv() {
            events.push(event);
        }
        events
    }

    pub fn shutdown(&self) {
        let _ = self.request("media.shutdown", json!({}));
        let deadline = Instant::now() + SHUTDOWN_TIMEOUT;
        loop {
            let exited = self
                .child
                .lock()
                .map(|mut child| child.try_wait().ok().flatten().is_some())
                .unwrap_or(true);
            if exited {
                return;
            }
            if Instant::now() >= deadline {
                self.kill();
                return;
            }
            thread::sleep(Duration::from_millis(25));
        }
    }

    pub fn is_running(&self) -> bool {
        self.child
            .lock()
            .map(|mut child| child.try_wait().ok().flatten().is_none())
            .unwrap_or(false)
    }

    fn kill(&self) {
        if let Ok(mut child) = self.child.lock() {
            if child.try_wait().ok().flatten().is_none() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }
}

impl Drop for MediaSupervisor {
    fn drop(&mut self) {
        self.kill();
    }
}

fn resolve_worker_executable() -> Result<PathBuf, MediaError> {
    if let Some(path) = env::var_os("HARBOR_MEDIA_EXECUTABLE") {
        return Ok(PathBuf::from(path));
    }
    let core = env::current_exe().map_err(|_| MediaError::ExecutableUnavailable)?;
    let parent = core.parent().ok_or(MediaError::ExecutableUnavailable)?;
    Ok(parent.join(media_binary_name()))
}

fn media_binary_name() -> &'static str {
    if cfg!(windows) {
        "harbor-media.exe"
    } else {
        "harbor-media"
    }
}

/// Drives one supervised child's IO. A dedicated reader thread owns stdout
/// for the child's whole life: Pion reports asynchronous facts (connection
/// state, ICE candidates) whenever they happen, and those must reach the
/// core even when no command is in flight — a `connected` event that waits
/// for the next request would leave the call stuck in CONNECTING forever.
/// Replies are demultiplexed from events by their correlation ids and
/// handed to the serialized command loop, which owns stdin.
fn run_worker(
    mut stdin: ChildStdin,
    stdout: ChildStdout,
    child: Arc<Mutex<Child>>,
    commands: mpsc::Receiver<MediaCommand>,
    events: mpsc::SyncSender<MediaEvent>,
    notify: Option<mpsc::Sender<()>>,
) {
    // Every exit path pokes the owner: queued facts and worker death are both
    // things the core must observe promptly, not at its next dispatch.
    let poke = |notify: &Option<mpsc::Sender<()>>| {
        if let Some(sender) = notify {
            let _ = sender.send(());
        }
    };
    let (replies, reply_receiver) = mpsc::channel::<Result<MediaReply, MediaError>>();

    let reader_notify = notify.clone();
    thread::spawn(move || {
        let mut stdout = BufReader::new(stdout);
        loop {
            let envelope = match read_frame(&mut stdout) {
                Ok(envelope) => envelope,
                Err(error) => {
                    // The worker died or spoke nonsense: fail the in-flight
                    // command, if any, and let the owner observe the exit.
                    let _ = replies.send(Err(error));
                    poke(&reader_notify);
                    return;
                }
            };
            if envelope.event_id.is_some() {
                if matches!(
                    envelope.message_type.as_str(),
                    "media.call_state"
                        | "media.ice_candidate"
                        | "media.transfer_update"
                        | "media.voice_level"
                        | "media.call_stats"
                ) {
                    let _ = events.try_send(MediaEvent {
                        message_type: envelope.message_type,
                        payload: envelope.payload,
                    });
                    poke(&reader_notify);
                }
                continue;
            }
            let _ = replies.send(Ok(MediaReply {
                message_type: envelope.message_type,
                payload: envelope.payload,
                error: envelope.error,
                reply_to: envelope.reply_to.unwrap_or_default(),
            }));
        }
    });

    while let Ok(command) = commands.recv() {
        let request_id = command.request.request_id.clone().unwrap_or_default();
        if write_frame(&mut stdin, &command.request).is_err() {
            let _ = command.reply.send(Err(MediaError::Io));
            poke(&notify);
            return;
        }
        match reply_receiver.recv_timeout(REQUEST_TIMEOUT) {
            Ok(Ok(reply)) => {
                if reply.reply_to != request_id {
                    // The worker answered something it was not asked; its
                    // stream can no longer be trusted. Dropping stdin ends
                    // the child; the supervisor marks it dead.
                    let _ = command.reply.send(Err(MediaError::UnexpectedReply));
                    poke(&notify);
                    return;
                }
                let _ = command.reply.send(Ok(reply));
            }
            Ok(Err(error)) => {
                let _ = command.reply.send(Err(error));
                poke(&notify);
                return;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                let _ = command.reply.send(Err(MediaError::TimedOut));
                poke(&notify);
                if let Ok(mut child) = child.lock() {
                    if child.try_wait().ok().flatten().is_none() {
                        let _ = child.kill();
                        let _ = child.wait();
                    }
                }
                return;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                let _ = command.reply.send(Err(MediaError::Exited));
                poke(&notify);
                return;
            }
        }
    }
    // stdin closed (graceful shutdown) or the queue was dropped: the child's
    // fate is decided by shutdown()/Drop, but the owner should re-check.
    poke(&notify);
}

fn write_frame<W: Write>(writer: &mut W, envelope: &MediaEnvelope) -> Result<(), MediaError> {
    let body = serde_json::to_vec(envelope).map_err(|_| MediaError::InvalidEnvelope)?;
    if body.is_empty() || body.len() > MEDIA_MAX_FRAME_BYTES {
        return Err(MediaError::InvalidEnvelope);
    }
    writer
        .write_all(&(body.len() as u32).to_be_bytes())
        .and_then(|_| writer.write_all(&body))
        .and_then(|_| writer.flush())
        .map_err(|_| MediaError::Io)
}

fn read_frame<R: Read>(reader: &mut R) -> Result<MediaEnvelope, MediaError> {
    let mut prefix = [0_u8; 4];
    reader.read_exact(&mut prefix).map_err(map_read_error)?;
    let length = u32::from_be_bytes(prefix) as usize;
    if length == 0 || length > MEDIA_MAX_FRAME_BYTES {
        return Err(MediaError::InvalidEnvelope);
    }
    let mut body = vec![0_u8; length];
    reader.read_exact(&mut body).map_err(map_read_error)?;
    let envelope: MediaEnvelope =
        serde_json::from_slice(&body).map_err(|_| MediaError::InvalidEnvelope)?;
    validate_worker_envelope(&envelope)?;
    Ok(envelope)
}

fn map_read_error(error: io::Error) -> MediaError {
    if error.kind() == io::ErrorKind::UnexpectedEof {
        MediaError::Exited
    } else {
        MediaError::Io
    }
}

fn validate_worker_envelope(envelope: &MediaEnvelope) -> Result<(), MediaError> {
    if envelope.version != MEDIA_PROTOCOL_VERSION
        || envelope.timestamp.is_empty()
        || !envelope.payload.is_object()
        || envelope
            .request_id
            .as_deref()
            .is_some_and(|id| Uuid::parse_str(id).is_err())
        || envelope
            .event_id
            .as_deref()
            .is_some_and(|id| Uuid::parse_str(id).is_err())
        || envelope
            .reply_to
            .as_deref()
            .is_some_and(|id| Uuid::parse_str(id).is_err())
        || envelope.request_id.is_some() == envelope.event_id.is_some()
    {
        return Err(MediaError::InvalidEnvelope);
    }
    if let Some(error) = &envelope.error {
        if error.code.is_empty() || error.ui_key.is_empty() {
            return Err(MediaError::InvalidEnvelope);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_frame_rejects_untrusted_correlation() {
        let envelope = MediaEnvelope {
            version: MEDIA_PROTOCOL_VERSION,
            message_type: "media.hello".into(),
            request_id: Some("not-a-uuid".into()),
            event_id: None,
            reply_to: None,
            timestamp: "2026-09-01T00:00:00Z".into(),
            payload: json!({}),
            error: None,
        };
        assert_eq!(
            validate_worker_envelope(&envelope),
            Err(MediaError::InvalidEnvelope)
        );
    }

    #[test]
    fn local_worker_name_follows_platform_convention() {
        assert!(media_binary_name().starts_with("harbor-media"));
    }
}
