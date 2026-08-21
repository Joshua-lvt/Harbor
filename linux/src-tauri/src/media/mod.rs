//! First Linux-native media slice.
//!
//! This module owns the safe Tauri boundary and GStreamer capture lifecycle.
//! WebRTC/DTLS/SRTP remains a later layer: signaling already crosses this
//! boundary as the same versioned offer/answer/ICE envelope used by the JS
//! peer. Keeping capture and IPC independent makes the JS WebRTC fallback safe
//! while the native peer session is brought up incrementally.

mod queue;
mod types;

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use base64::Engine;
use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use tauri::{AppHandle, Emitter, State};

use queue::BoundedQueue;
pub use types::{
    MediaCapabilities, MediaSignalEnvelope, MediaStartRequest, MediaStateEvent, MEDIA_AUDIO_EVENT,
    MEDIA_PROTOCOL_VERSION, MEDIA_STATE_EVENT, MEDIA_VIDEO_EVENT,
};
use types::{MediaFrameEvent, MAX_FRAME_BYTES};

const AUDIO_QUEUE_CAPACITY: usize = 8;
const VIDEO_QUEUE_CAPACITY: usize = 3;
const SIGNAL_QUEUE_CAPACITY: usize = 32;
const WORKER_POLL: Duration = Duration::from_millis(100);

#[derive(Debug)]
struct RawFrame {
    timestamp_ns: u64,
    bytes: Vec<u8>,
}

pub struct MediaRuntime {
    session: Mutex<Option<MediaSession>>,
    signals: Arc<BoundedQueue<MediaSignalEnvelope>>,
    ptt: Arc<AtomicBool>,
}

impl Default for MediaRuntime {
    fn default() -> Self {
        Self {
            session: Mutex::new(None),
            signals: Arc::new(BoundedQueue::new(SIGNAL_QUEUE_CAPACITY)),
            ptt: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl MediaRuntime {
    pub fn stop(&self, app: &AppHandle) -> Result<(), String> {
        self.ptt.store(false, Ordering::Release);
        let session = self
            .session
            .lock()
            .map_err(|_| "media state poisoned")?
            .take();
        if let Some(session) = session {
            session.stop();
        }
        self.signals.clear();
        emit_state(app, "stopped", "none", None);
        Ok(())
    }
}

#[derive(Debug)]
struct MediaSession {
    room_id: String,
    local_device_id: String,
    partner_device_id: String,
    pipelines: Vec<gst::Pipeline>,
    stop: Arc<AtomicBool>,
    workers: Vec<JoinHandle<()>>,
}

impl MediaSession {
    fn accepts_signal_fields(
        room_id: &str,
        local_device_id: &str,
        partner_device_id: &str,
        signal: &MediaSignalEnvelope,
    ) -> bool {
        signal.room_id == room_id
            && signal.sender_id == partner_device_id
            && signal.sender_id != local_device_id
    }

    fn accepts_signal(&self, signal: &MediaSignalEnvelope) -> bool {
        Self::accepts_signal_fields(
            &self.room_id,
            &self.local_device_id,
            &self.partner_device_id,
            signal,
        )
    }

    fn stop(mut self) {
        self.stop.store(true, Ordering::Release);
        for pipeline in &self.pipelines {
            let _ = pipeline.set_state(gst::State::Null);
        }
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
    }
}

impl Drop for MediaRuntime {
    fn drop(&mut self) {
        if let Ok(mut guard) = self.session.lock() {
            if let Some(session) = guard.take() {
                session.stop();
            }
        }
    }
}

fn ensure_gstreamer() -> Result<(), String> {
    gst::init().map_err(|error| format!("GStreamer initialization failed: {error}"))
}

fn factory_available(name: &str) -> bool {
    gst::ElementFactory::find(name).is_some()
}

fn audio_backend() -> Option<&'static str> {
    if factory_available("pipewiresrc") {
        Some("pipewire")
    } else if factory_available("pulsesrc") {
        Some("pulseaudio")
    } else {
        None
    }
}

fn screen_backend() -> Option<&'static str> {
    let force_x11 = std::env::var("HARBOR_FORCE_X11").is_ok_and(|value| value == "1");
    let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some();
    let x11 = std::env::var_os("DISPLAY").is_some();

    if !force_x11 && wayland && factory_available("pipewiresrc") {
        // The portal-selected PipeWire node is supplied by the desktop portal
        // integration. This first slice keeps the source/pipeline bounded; the
        // portal session hand-off is added without changing this IPC contract.
        Some("pipewire")
    } else if x11 && factory_available("ximagesrc") {
        Some("x11")
    } else {
        None
    }
}

pub fn capabilities() -> Result<MediaCapabilities, String> {
    ensure_gstreamer()?;
    let audio = audio_backend();
    let screen = screen_backend();
    Ok(MediaCapabilities {
        protocol_version: MEDIA_PROTOCOL_VERSION,
        backend: "gstreamer".to_string(),
        audio_capture: audio.is_some(),
        screen_capture: screen.is_some(),
        screen_backend: screen.unwrap_or("unavailable").to_string(),
        // Deliberately false until we add the webrtc-rs peer session. The JS
        // WebRTC engine remains the production fallback in this slice.
        native_webrtc: false,
    })
}

fn build_pipeline(description: &str) -> Result<gst::Pipeline, String> {
    gst::parse::launch(description)
        .map_err(|error| format!("GStreamer pipeline parse failed: {error}"))?
        .downcast::<gst::Pipeline>()
        .map_err(|_| "GStreamer did not return a pipeline".to_string())
}

fn next_monotonic_timestamp(last: &AtomicU64, candidate: u64) -> u64 {
    let mut observed = last.load(Ordering::Acquire);
    loop {
        let next = candidate.max(observed.saturating_add(1));
        match last.compare_exchange_weak(observed, next, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => return next,
            Err(current) => observed = current,
        }
    }
}

fn install_sink_callback(
    pipeline: &gst::Pipeline,
    sink_name: &str,
    queue: Arc<BoundedQueue<RawFrame>>,
    dropped: Arc<AtomicU64>,
    last_timestamp: Arc<AtomicU64>,
    fallback_origin: Instant,
) -> Result<(), String> {
    let sink = pipeline
        .by_name(sink_name)
        .ok_or_else(|| format!("missing GStreamer sink {sink_name}"))?
        .downcast::<gst_app::AppSink>()
        .map_err(|_| format!("{sink_name} is not an appsink"))?;

    let callbacks = gst_app::AppSinkCallbacks::builder()
        .new_sample(move |sink| {
            let sample = sink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
            let Some(buffer) = sample.buffer() else {
                return Ok(gst::FlowSuccess::Ok);
            };
            let Some(map) = buffer.map_readable().ok() else {
                return Ok(gst::FlowSuccess::Ok);
            };
            if map.size() > MAX_FRAME_BYTES {
                dropped.fetch_add(1, Ordering::Relaxed);
                return Ok(gst::FlowSuccess::Ok);
            }
            let candidate_timestamp =
                buffer.pts().map(|time| time.nseconds()).unwrap_or_else(|| {
                    fallback_origin.elapsed().as_nanos().min(u64::MAX as u128) as u64
                });
            let frame = RawFrame {
                // GStreamer PTS can be missing or repeat during source changes;
                // normalize it before enqueueing so consumers can discard stale
                // frames without seeing time move backwards.
                timestamp_ns: next_monotonic_timestamp(&last_timestamp, candidate_timestamp),
                bytes: map.as_slice().to_vec(),
            };
            if queue.push_drop_oldest(frame) {
                dropped.fetch_add(1, Ordering::Relaxed);
            }
            Ok(gst::FlowSuccess::Ok)
        })
        .build();
    sink.set_callbacks(callbacks);
    Ok(())
}

fn audio_description(source: &str) -> String {
    format!(
        "{source} do-timestamp=true ! audioconvert ! audioresample ! audio/x-raw,format=S16LE,channels=1,rate=48000 ! queue max-size-buffers=8 max-size-bytes=65536 max-size-time=200000000 leaky=downstream ! appsink name=audio_sink emit-signals=false max-buffers=8 drop=true sync=false"
    )
}

fn screen_description(source: &str) -> String {
    let source = match source {
        "pipewire" => "pipewiresrc do-timestamp=true".to_string(),
        // X11 is intentionally explicit and never selected on Wayland unless
        // the operator opted into the fallback through HARBOR_FORCE_X11.
        "x11" => "ximagesrc use-damage=false show-pointer=true".to_string(),
        _ => unreachable!("validated screen source"),
    };
    format!(
        "{source} ! videoconvert ! videoscale ! video/x-raw,format=RGBA,width=1280,height=720,framerate=15/1 ! queue max-size-buffers=3 max-size-bytes=7864320 max-size-time=250000000 leaky=downstream ! appsink name=video_sink emit-signals=false max-buffers=3 drop=true sync=false"
    )
}

fn spawn_frame_worker(
    app: AppHandle,
    event: &'static str,
    queue: Arc<BoundedQueue<RawFrame>>,
    stop: Arc<AtomicBool>,
    dropped: Arc<AtomicU64>,
    ptt_gate: Option<Arc<AtomicBool>>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        let encoder = base64::engine::general_purpose::STANDARD;
        while !stop.load(Ordering::Acquire) {
            let Some(frame) = queue.pop_timeout(WORKER_POLL) else {
                continue;
            };
            if ptt_gate
                .as_ref()
                .is_some_and(|gate| !gate.load(Ordering::Acquire))
            {
                continue;
            }
            let event_payload = MediaFrameEvent {
                timestamp_ns: frame.timestamp_ns,
                data_base64: encoder.encode(frame.bytes),
                dropped_before_emit: dropped.swap(0, Ordering::Relaxed),
            };
            let _ = app.emit(event, event_payload);
        }
    })
}

fn emit_state(app: &AppHandle, state: &str, backend: &str, detail: Option<String>) {
    let _ = app.emit(
        MEDIA_STATE_EVENT,
        MediaStateEvent {
            state: state.to_string(),
            backend: backend.to_string(),
            detail,
        },
    );
}

#[tauri::command]
pub fn media_capabilities() -> Result<MediaCapabilities, String> {
    capabilities()
}

#[tauri::command]
pub fn media_start(
    app: AppHandle,
    runtime: State<'_, MediaRuntime>,
    request: MediaStartRequest,
) -> Result<MediaCapabilities, String> {
    request.validate()?;
    ensure_gstreamer()?;
    let mut guard = runtime
        .session
        .lock()
        .map_err(|_| "media state poisoned".to_string())?;
    if guard.is_some() {
        return Err("native media is already running".to_string());
    }
    // Signals are ephemeral and scoped to the previous session. Discard any
    // queued values before accepting a new room/session identity.
    runtime.signals.clear();
    runtime.ptt.store(false, Ordering::Release);

    let audio_name = audio_backend().ok_or_else(|| {
        "GStreamer audio capture is unavailable; install PipeWire or PulseAudio plugins".to_string()
    })?;
    let capture_origin = Instant::now();
    let audio_queue = Arc::new(BoundedQueue::new(AUDIO_QUEUE_CAPACITY));
    let audio_dropped = Arc::new(AtomicU64::new(0));
    let audio_pipeline = build_pipeline(&audio_description(match audio_name {
        "pipewire" => "pipewiresrc",
        "pulseaudio" => "pulsesrc",
        _ => unreachable!(),
    }))?;
    install_sink_callback(
        &audio_pipeline,
        "audio_sink",
        Arc::clone(&audio_queue),
        Arc::clone(&audio_dropped),
        Arc::new(AtomicU64::new(0)),
        capture_origin,
    )?;

    let screen_name = if request.screen {
        Some(screen_backend().ok_or_else(|| {
            "screen capture is unavailable; use a portal-enabled Wayland session or X11".to_string()
        })?)
    } else {
        None
    };
    let screen_pair = if let Some(name) = screen_name {
        let queue = Arc::new(BoundedQueue::new(VIDEO_QUEUE_CAPACITY));
        let dropped = Arc::new(AtomicU64::new(0));
        let pipeline = build_pipeline(&screen_description(name))?;
        install_sink_callback(
            &pipeline,
            "video_sink",
            Arc::clone(&queue),
            Arc::clone(&dropped),
            Arc::new(AtomicU64::new(0)),
            capture_origin,
        )?;
        Some((pipeline, queue, dropped))
    } else {
        None
    };

    let stop = Arc::new(AtomicBool::new(false));
    audio_pipeline
        .set_state(gst::State::Playing)
        .map_err(|error| format!("could not start audio capture: {error:?}"))?;
    let mut pipelines = vec![audio_pipeline];
    let mut workers = vec![spawn_frame_worker(
        app.clone(),
        MEDIA_AUDIO_EVENT,
        audio_queue,
        Arc::clone(&stop),
        audio_dropped,
        Some(Arc::clone(&runtime.ptt)),
    )];
    if let Some((pipeline, queue, dropped)) = screen_pair {
        pipeline
            .set_state(gst::State::Playing)
            .map_err(|error| format!("could not start screen capture: {error:?}"))?;
        pipelines.push(pipeline);
        workers.push(spawn_frame_worker(
            app.clone(),
            MEDIA_VIDEO_EVENT,
            queue,
            Arc::clone(&stop),
            dropped,
            None,
        ));
    }

    *guard = Some(MediaSession {
        room_id: request.room_id,
        local_device_id: request.device_id,
        partner_device_id: request.partner_id,
        pipelines,
        stop,
        workers,
    });
    let caps = capabilities()?;
    emit_state(
        &app,
        "capturing",
        audio_name,
        screen_name.map(str::to_string),
    );
    Ok(caps)
}

#[tauri::command]
pub fn media_stop(app: AppHandle, runtime: State<'_, MediaRuntime>) -> Result<(), String> {
    runtime.stop(&app)
}

#[tauri::command]
pub fn media_set_ptt(
    app: AppHandle,
    runtime: State<'_, MediaRuntime>,
    active: bool,
) -> Result<(), String> {
    runtime.ptt.store(active, Ordering::Release);
    emit_state(
        &app,
        if active { "ptt_active" } else { "ptt_inactive" },
        "gstreamer",
        None,
    );
    Ok(())
}

#[tauri::command]
pub fn media_receive_signal(
    runtime: State<'_, MediaRuntime>,
    signal: MediaSignalEnvelope,
) -> Result<(), String> {
    signal.validate()?;
    let guard = runtime
        .session
        .lock()
        .map_err(|_| "media state poisoned".to_string())?;
    let Some(session) = guard.as_ref() else {
        return Err("native media is not running".to_string());
    };
    if !session.accepts_signal(&signal) {
        return Err("media signal is not addressed to the active session".to_string());
    }
    runtime.signals.push_drop_oldest(signal);
    Ok(())
}

/// Kept private for now: the future webrtc-rs session consumes this bounded
/// queue. Exposing the queue itself over IPC would bypass the payload limits.
#[allow(dead_code)]
pub(crate) fn take_signal(runtime: &MediaRuntime) -> Option<MediaSignalEnvelope> {
    runtime.signals.pop_timeout(Duration::ZERO)
}

#[cfg(test)]
mod tests {
    use super::{audio_description, next_monotonic_timestamp, screen_description, MediaSession};
    use crate::media::types::{MediaSignalEnvelope, MediaStartRequest};
    use serde_json::json;
    use std::sync::atomic::AtomicU64;

    fn session_request() -> MediaStartRequest {
        MediaStartRequest {
            room_id: "pair:a:b".to_string(),
            device_id: "a".to_string(),
            partner_id: "b".to_string(),
            screen: false,
        }
    }

    fn signal(room_id: &str, sender_id: &str) -> MediaSignalEnvelope {
        MediaSignalEnvelope {
            v: 1,
            room_id: room_id.to_string(),
            sender_id: sender_id.to_string(),
            seq: 1,
            kind: "ice".to_string(),
            data: json!({"candidate": "candidate:1"}),
            sent_at: 1,
        }
    }

    #[test]
    fn accepts_only_the_other_device_in_the_active_room() {
        let request = session_request();
        assert!(MediaSession::accepts_signal_fields(
            &request.room_id,
            &request.device_id,
            &request.partner_id,
            &signal("pair:a:b", "b")
        ));
        assert!(!MediaSession::accepts_signal_fields(
            &request.room_id,
            &request.device_id,
            &request.partner_id,
            &signal("pair:a:c", "b")
        ));
        assert!(!MediaSession::accepts_signal_fields(
            &request.room_id,
            &request.device_id,
            &request.partner_id,
            &signal("pair:a:b", "a")
        ));
        assert!(!MediaSession::accepts_signal_fields(
            &request.room_id,
            &request.device_id,
            &request.partner_id,
            &signal("pair:a:b", "c")
        ));
    }

    #[test]
    fn normalizes_timestamps_to_strictly_increasing_values() {
        let last = AtomicU64::new(0);
        assert_eq!(next_monotonic_timestamp(&last, 0), 1);
        assert_eq!(next_monotonic_timestamp(&last, 1), 2);
        assert_eq!(next_monotonic_timestamp(&last, 10), 10);
        assert_eq!(next_monotonic_timestamp(&last, 9), 11);
    }

    #[test]
    fn pipelines_have_bounded_queues_and_drop_stale_frames() {
        let audio = audio_description("pipewiresrc");
        let screen = screen_description("x11");
        assert!(audio.contains("max-size-buffers=8"));
        assert!(audio.contains("leaky=downstream"));
        assert!(screen.contains("max-size-buffers=3"));
        assert!(screen.contains("framerate=15/1"));
    }
}
