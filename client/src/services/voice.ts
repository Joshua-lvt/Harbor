/**
 * Ambient voice channel between the two paired devices — a WebRTC audio-only
 * peer connection, signaled through the relay's `voice_signal` envelopes
 * (offer/answer/ICE). Audio itself is P2P; the relay only relays signaling.
 *
 * Push-to-talk: the mic is muted by default; hold Left Alt (VK_LMENU) to speak,
 * polled via the Rust `is_key_pressed` command. This keeps the channel quiet
 * unless someone actively talks — "Estamos juntos, mesmo longe".
 *
 * Singleton like `ws.ts`: created once (module scope), survives route changes so
 * a call stays up when you open Chat. A hidden remote <audio> lives in App.tsx
 * (`attachAudioElement`); HomeScreen's `CallStrip` reads state via `hooks/useVoice.ts`.
 *
 * ALWAYS-ON MODEL (no "iniciar"/"sair"):
 * The call has no start/hang-up button anymore. While paired, App calls
 * `startAlwaysOn(isOfferer)` exactly once; the singleton then drives itself off the
 * socket status (it subscribes to `socket.onStatus` internally). Role is
 * deterministic — the smaller `device_id` offers — so both sides never offer at once
 * (no glare). The offerer keeps re-offering until connected; the responder sits in
 * `reconnecting` and auto-answers inbound offers (`onOffer`).
 *
 * First-run mic permission: `getUserMedia` needs a user gesture the first time. On
 * a fresh install the optimistic `ensureMic()` in the open-handler yields
 * `NotAllowedError` → status `needs_permission`, and the `CallStrip` shows a
 * "Permitir microfone" button; that click is the gesture (`grantAndConnect`).
 * WebView2 keeps the grant across launches, so from then on the open-handler's
 * `getUserMedia` resolves silently and the call reconnects itself on every boot
 * with no button at all — exactly the "sempre em call" the product wants.
 *
 * Mic-blocked fallback: if the click STILL yields NotAllowedError (Windows mic
 * privacy is off for the app, or WebView2 silently denies — its
 * PermissionRequested event isn't reliably fired for the mic, see issue #1462),
 * `ensureMic(true)` flips to `mic_blocked` instead of looping. That state's
 * action opens the Windows mic settings (`open_microphone_settings` Rust
 * command); after toggling, "Tentar novamente" re-attempts. Without this the
 * button would deny-and-reappear unchanged — "nothing happens".
 *
 * MVP limits (documented honestly):
 *  - A single public STUN server; no TURN relay, so peers behind symmetric NAT
 *    may fail to connect cross-network (works on LAN/Tailscale). TURN is future.
 *  - Signaling is transient (forward-only on the relay). The offerer's ~8s retry
 *    covers an offline/late partner; there is no "missed call" buffering.
 */
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { socket } from "./ws";
import type { ServerEvent } from "../lib/types";

const ICE_SERVERS = [{ urls: "stun:stun.l.google.com:19302" }];
const VK_LMENU = 0xa4; // Left Alt — Push-to-Talk
const PTT_POLL_MS = 120;
/** Offerer re-offers on this cadence while waiting to connect. */
const RECONNECT_RETRY_MS = 8_000;
/** If negotiation doesn't reach `connected` in this window, tear down and retry. */
const CONNECTING_TIMEOUT_MS = 20_000;

export type VoiceStatus =
  | "needs_permission"
  | "mic_blocked"
  | "connecting"
  | "connected"
  | "reconnecting"
  | "failed";

export interface VoiceState {
  status: VoiceStatus;
  /** Mic currently hot (Left Alt held). */
  pttActive: boolean;
  /** The partner's audio is currently above the speech threshold — lights the
   *  blue ring around their avatar in the call strip. Derived from an
   *  AnalyserNode on the remote stream (see startSpeakingDetect). Stays false
   *  if the Web Audio analysis path is unavailable (honest degradation). */
  partnerSpeaking: boolean;
  error: string | null;
}

type Listener = (s: VoiceState) => void;

class VoiceManager {
  private pc: RTCPeerConnection | null = null;
  private localStream: MediaStream | null = null;
  private remoteStream: MediaStream | null = null;
  private state: VoiceState = {
    status: "reconnecting",
    pttActive: false,
    partnerSpeaking: false,
    error: null,
  };
  private listeners = new Set<Listener>();
  private audioEls = new Set<HTMLAudioElement>();
  private pttTimer: number | null = null;
  // Partner-speaking analysis: an AudioContext + AnalyserNode tapping the
  // remote stream, polled for RMS. Null when not analysing (idle / failed, or
  // when the Web Audio APIs are unavailable — degrade silently).
  private audioCtx: AudioContext | null = null;
  private analyser: AnalyserNode | null = null;
  private speakTimer: number | null = null;

  // --- always-on driver ----------------------------------------------------

  /** Whether this device is the offerer (smaller device_id). Set before
   *  `startAlwaysOn` so the open-handler knows whether to offer or wait. */
  private isOfferer = false;
  /** True while the always-on driver is active (between `startAlwaysOn` and
   *  `stopAlwaysOn`). Gates the socket-status subscriber + retry/timeout timers
   *  so a stopped singleton doesn't keep reconnecting against a dead link. */
  private running = false;
  /** Offerer re-offer cadence. Set in `scheduleRetry`, cleared when leaving
   *  `reconnecting` or on `stopAlwaysOn`. */
  private retryTimer: number | null = null;
  /** Watchdog that demotes a stuck `connecting` back to `reconnecting`. */
  private connectingTimer: number | null = null;
  /** Unsubscribe handle for the socket-status subscription owned here. */
  private offStatus: (() => void) | null = null;

  constructor() {
    // Wire signaling to the app-lifetime socket singleton. We never unsubscribe
    // — voice lasts the app lifetime, same as the socket. Inbound voice_signal
    // envelopes are routed here; outbound signaling rides the same socket.
    socket.onEvent((e: ServerEvent) => {
      if (e.type !== "voice_signal") return;
      void this.onSignal(e.kind, e.data);
    });
    // Re-attempt the mic when the main window regains focus while blocked. The
    // only recovery from `mic_blocked` is the user toggling the Windows privacy
    // setting (opened via `open_microphone_settings`); when they come back to
    // the app, re-run grantAndConnect so the call lifts off without forcing them
    // to click "Tentar novamente". The unsub handle is intentionally discarded
    // — voice is an app-lifetime singleton, so the listener lives as long as the
    // app. Best-effort; fails silently on non-Tauri builds.
    try {
      void getCurrentWindow()
        .onFocusChanged(({ payload: focused }) => {
          if (focused && this.state.status === "mic_blocked" && this.running) {
            void this.grantAndConnect();
          }
        })
        .catch(() => {});
    } catch {
      // non-Tauri / window unavailable — no-op
    }
  }

  /** Subscribe to state; returns unsubscribe. Runs the listener once immediately. */
  onState(l: Listener): () => void {
    this.listeners.add(l);
    l(this.state);
    return () => this.listeners.delete(l);
  }

  getState(): VoiceState {
    return this.state;
  }

  /** Set the deterministic role. Must be called before `startAlwaysOn` so the
   *  first open-handler routes to the right side. Idempotent. */
  setRole(isOfferer: boolean): void {
    this.isOfferer = isOfferer;
  }

  /** Engage the always-on driver. Subscribes to socket status; on every `open`
   *  it (re)negotiates per the role. Call exactly once when paired; pair with
   *  `stopAlwaysOn` on unpair. Re-calling while running is a no-op-ish refresh
   *  of the role (kept simple — unpair calls stopAlwaysOn first anyway). */
  startAlwaysOn(isOfferer: boolean): void {
    this.setRole(isOfferer);
    if (this.running) return;
    this.running = true;
    if (this.offStatus) this.offStatus();
    this.offStatus = socket.onStatus((s) => this.onSocket(s));
    // If the socket is already open, kick the open handler now (covers the
    // boot path where the socket connected before the voice driver started).
    if (socket.getStatus() === "open") void this.onSocket("open");
  }

  /** Tear down the always-on driver entirely (called on unpair). Cancels timers,
   *  tears down the PC, releases the mic, detaches from the socket. The singleton
   *  is left in a stopped state; a later `startAlwaysOn` re-engages it. */
  stopAlwaysOn(): void {
    this.running = false;
    if (this.offStatus) {
      this.offStatus();
      this.offStatus = null;
    }
    this.clearRetry();
    this.clearConnectingTimer();
    this.teardownPC();
    this.releaseMic();
    // Stopped: distinct from reconnecting — no socket, no retry, no mic. The
    // listeners (CallStrip) keep a stale-but-harmless status until the next
    // startAlwaysOn; the screen that shows them is gone by then anyway.
    this.set({ status: "reconnecting", pttActive: false, partnerSpeaking: false, error: null });
  }

  /** The one user gesture in the whole flow: the "Permitir microfone" /
   *  "Tentar novamente" click. `fromGesture=true` tells ensureMic this IS a
   *  user gesture, so a NotAllowedError here is the OS/WebView2 blocking the
   *  mic (not a missing gesture) and surfaces as `mic_blocked` rather than
   *  looping silently back to "Permitir microfone" forever. */
  async grantAndConnect(): Promise<void> {
    const ok = await this.ensureMic(true);
    if (!ok) return; // ensureMic set the appropriate failed/needs_permission/mic_blocked state
    if (!this.running) return;
    // Move to "reconnecting" BEFORE branching on role. connect() guards on
    // `status === "reconnecting"` — and on first run the status was
    // "needs_permission", so WITHOUT this the offerer's connect() early-returned
    // and the click did literally nothing ("Permitir microfone" não fazia nada).
    // The responder needs this too (its onOffer path refuses anything that
    // isn't "reconnecting").
    this.set({ status: "reconnecting" });
    if (this.isOfferer) {
      this.connect();
    }
    // Responder: "reconnecting" is enough — the offerer keeps re-offering every
    // RECONNECT_RETRY_MS, and onOffer auto-answers the next inbound offer.
  }

  /** Attach a remote <audio> element to play the peer's stream (App renders
   *  one hidden, persistent across routes). */
  attachAudioElement(el: HTMLAudioElement): () => void {
    this.audioEls.add(el);
    this._playOn(el, this.remoteStream);
    return () => {
      this.audioEls.delete(el);
      el.srcObject = null;
    };
  }

  /** Attach a stream to an <audio> element and start it. `autoPlay` alone is
   *  unreliable in WebView2 — the remote audio can connect yet play silently,
   *  so the call shows "connected" but the partner is inaudible. We explicitly
   *  call play(); the rejection is swallowed because the stream may not be
   *  ready yet (ontrack re-attaches once it is). */
  private _playOn(el: HTMLAudioElement, stream: MediaStream | null): void {
    el.srcObject = stream;
    if (stream) void el.play().catch(() => {});
  }

  // --- socket-status driver -------------------------------------------------

  private onSocket(s: "connecting" | "open" | "closed"): void {
    if (!this.running) return;
    if (s === "open") {
      void this.onSocketOpen();
    } else {
      // `connecting`/`closed`: drop the PC (keep the mic, keep it muted) and
      // wait for the next `open`. If there's no mic yet there's nothing to
      // reconnect with — show needs_permission so the gesture button appears.
      this.teardownPC();
      this.clearRetry();
      this.clearConnectingTimer();
      this.setPtt(false);
      this.set({
        status: this.localStream ? "reconnecting" : "needs_permission",
        partnerSpeaking: false,
      });
    }
  }

  private async onSocketOpen(): Promise<void> {
    if (!this.running) return;
    // Optimistically (re)acquire the mic. On a permission granted in a prior
    // session this resolves immediately (WebView2 keeps the grant); it's the
    // "no button on boot" path. On a fresh install it rejects with
    // NotAllowedError → needs_permission + the gesture button.
    const mic = await this.ensureMic();
    if (!this.running) return; // stopAlwaysOn raced in during the await
    if (!mic) return; // ensureMic set needs_permission/failed
    if (this.isOfferer) {
      this.connect();
    } else {
      this.set({ status: "reconnecting" });
      this.scheduleRetry();
    }
  }

  // --- negotiation ----------------------------------------------------------

  /** Offerer: create PC + local offer + send it. Guarded to `reconnecting` and
   *  `this.pc === null` so a stuck teardown can't pile a fresh PC on an old one. */
  private async connect(): Promise<void> {
    if (!this.running) return;
    if (this.state.status !== "reconnecting") return;
    if (this.pc) return;
    this.set({ status: "connecting", error: null, partnerSpeaking: false });
    this.createPC();
    this.startConnectingTimer();
    try {
      const offer = await this.pc!.createOffer();
      await this.pc!.setLocalDescription(offer);
      this.sendSignal("offer", offer);
    } catch {
      // Build failed — go back to reconnecting so the retry loop fires again.
      this.clearConnectingTimer();
      this.teardownPC();
      this.set({ status: "reconnecting" });
      this.scheduleRetry();
    }
  }

  // --- signaling ------------------------------------------------------------

  private sendSignal(kind: "offer" | "answer" | "ice", data: unknown): void {
    socket.send({ type: "voice_signal", kind, data });
  }

  private async onSignal(kind: string, data: unknown): Promise<void> {
    if (kind === "offer") await this.onOffer(data as RTCSessionDescriptionInit);
    else if (kind === "answer") await this.onAnswer(data as RTCSessionDescriptionInit);
    else if (kind === "ice") await this.onIce(data as RTCIceCandidateInit);
  }

  /** Inbound offer → auto-answer (the offerer opened the call). Only while the
   *  responder is `reconnecting` — a retry offer racing an in-flight negotiation
   *  is dropped; the in-flight one either completes or times out, and the next
   *  retry is picked up after we fall back to `reconnecting`. Also refuses to
   *  prompt for the mic without a gesture: no localStream → needs_permission. */
  private async onOffer(offer: RTCSessionDescriptionInit): Promise<void> {
    if (!this.running) return;
    if (this.state.status !== "reconnecting") return;
    if (!this.localStream) {
      this.set({ status: "needs_permission" });
      return;
    }
    this.set({ status: "connecting", error: null, partnerSpeaking: false });
    this.createPC();
    this.startConnectingTimer();
    try {
      await this.pc!.setRemoteDescription(offer);
      const answer = await this.pc!.createAnswer();
      await this.pc!.setLocalDescription(answer);
      this.sendSignal("answer", answer);
    } catch {
      this.clearConnectingTimer();
      this.teardownPC();
      this.set({ status: "reconnecting" });
      this.scheduleRetry();
    }
  }

  private async onAnswer(answer: RTCSessionDescriptionInit): Promise<void> {
    if (!this.pc || this.pc.signalingState !== "have-local-offer") return;
    try {
      await this.pc.setRemoteDescription(answer);
    } catch {
      // stale/dupe — ignore
    }
  }

  private async onIce(candidate: RTCIceCandidateInit): Promise<void> {
    if (!this.pc) return;
    try {
      await this.pc.addIceCandidate(candidate);
    } catch {
      // ignore — often a race with setRemoteDescription
    }
  }

  // --- peer connection + PTT ------------------------------------------------

  private createPC(): void {
    const pc = new RTCPeerConnection({ iceServers: ICE_SERVERS });
    // Add the local mic track (muted until PTT raises it).
    this.localStream?.getAudioTracks().forEach((t) => pc.addTrack(t, this.localStream!));
    this.setMicTracksEnabled(false);

    pc.onicecandidate = (ev) => {
      if (ev.candidate) this.sendSignal("ice", ev.candidate);
    };
    pc.ontrack = (ev) => {
      // The peer's audio stream arrived — play it, mark connected, and start
      // analysing it for the partner-speaking ring (Discord-style "who's
      // talking"). startSpeakingDetect degrades silently if Web Audio is
      // unavailable — audio still plays via the <audio> element either way.
      this.remoteStream = ev.streams[0] ?? new MediaStream([ev.track]);
      this.audioEls.forEach((el) => this._playOn(el, this.remoteStream));
      this.startSpeakingDetect(this.remoteStream);
      this.clearConnectingTimer();
      this.clearRetry();
      this.set({ status: "connected" });
    };
    pc.onconnectionstatechange = () => {
      const s = pc.connectionState;
      if (s === "failed") {
        // A failed connection can't carry audio, so the partner is by
        // definition not speaking anymore — reset the ring alongside the
        // reconnect, and tear down the PC so the retry builds a fresh one.
        this.clearConnectingTimer();
        this.teardownPC();
        this.setPtt(false);
        this.set({ status: "reconnecting", partnerSpeaking: false });
        this.scheduleRetry();
      } else if (
        (s === "disconnected" || s === "closed") &&
        this.state.status === "connected"
      ) {
        // Peer left (or the link dropped for good) — back to reconnecting; the
        // offerer's retry loop (or an inbound offer, on the responder) lifts us
        // back up without any user action.
        this.teardownPC();
        this.setPtt(false);
        this.set({ status: "reconnecting", partnerSpeaking: false });
        this.scheduleRetry();
      }
    };
    this.pc = pc;
    this.startPTT();
  }

  /** Get the local mic, surfacing a clear error keyed to the failure mode.
   *  getUserMedia's exception names are standardized DOMException types, so we
   *  can tell the user exactly what went wrong instead of blaming permission
   *  for every failure (which hid "no mic" / "mic busy" behind the wrong message).
   *  `needs_permission` is the *recoverable* no-gesture-yet state (first-run
   *  optimistic call); `mic_blocked` is the recoverable "gesture was given but
   *  the OS/WebView2 still denies the mic" state (opens Windows mic settings);
   *  `failed` is the terminal/unrecoverable one (mic busy, no device).
   *
   *  `fromGesture` distinguishes a NotAllowedError that came from the click
   *  (→ `mic_blocked`: the OS privacy toggle is the only lever left) from one
   *  that came from the optimistic boot path (→ `needs_permission`: just show
   *  the button). Without this split, the "Permitir microfone" click would
   *  deny-and-loop back to the same button with no visible change — the
   *  "nothing happens" the user sees. */
  private async ensureMic(fromGesture = false): Promise<boolean> {
    if (this.localStream) return true;
    try {
      this.localStream = await navigator.mediaDevices.getUserMedia({ audio: true });
      return true;
    } catch (e) {
      const name = (e as DOMException)?.name;
      if (name === "NotAllowedError" || name === "SecurityError") {
        if (fromGesture) {
          // A user gesture was given (the click) and the mic STILL won't open.
          // This is NOT the missing-gesture first-run path — it's Windows mic
          // privacy blocking the app, or WebView2 silently denying the
          // microphone (its PermissionRequested event isn't reliably fired for
          // it — see MicrosoftEdge/WebView2Feedback#1462). Staying at the button
          // forever looks stuck, so surface a state whose action opens the OS
          // settings; the user toggles access, returns, and hits "Tentar
          // novamente".
          this.set({
            status: "mic_blocked",
            error: "O Windows está bloqueando o microfone para o Harbor.",
          });
        } else {
          // No user gesture on this boot yet, or the user declined. Recoverable
          // via the "Permitir microfone" button.
          this.set({ status: "needs_permission", error: null });
        }
        return false;
      }
      // Everything else (NotFound, NotReadable = mic busy, Overconstrained, …)
      // is a real, terminal failure for this session — surface it so the user
      // sees what's wrong and can "Tentar novamente" (or fix the device).
      let error = "Falha ao acessar o microfone";
      if (name === "NotFoundError" || name === "OverconstrainedError")
        error = "Nenhum microfone encontrado";
      else if (name === "NotReadableError") error = "Microfone em uso por outro app";
      this.set({ status: "failed", error });
      return false;
    }
  }

  private setMicTracksEnabled(on: boolean): void {
    this.localStream?.getAudioTracks().forEach((t) => (t.enabled = on));
  }

  /** Poll Left Alt while a PC exists → toggle the mic (Push-to-Talk). */
  private startPTT(): void {
    this.stopPTT();
    let ptt = false;
    const tick = async () => {
      let pressed = false;
      try {
        pressed = !!(await invoke<boolean>("is_key_pressed", { vkCode: VK_LMENU }));
      } catch {
        pressed = false; // command unavailable (non-Windows) → mic stays muted
      }
      if (pressed !== ptt) {
        ptt = pressed;
        this.setMicTracksEnabled(ptt);
        this.set({ pttActive: ptt });
      }
    };
    this.pttTimer = window.setInterval(tick, PTT_POLL_MS);
    void tick();
  }

  private stopPTT(): void {
    if (this.pttTimer != null) {
      clearInterval(this.pttTimer);
      this.pttTimer = null;
    }
    this.setMicTracksEnabled(false);
  }

  /** Push the PTT state down without restarting the poll (used on teardown). */
  private setPtt(active: boolean): void {
    this.setMicTracksEnabled(active);
    if (this.state.pttActive !== active) this.set({ pttActive: active });
  }

  // --- partner-speaking detection ------------------------------------------
  // Tap the remote stream with an AnalyserNode and poll its RMS. Above a
  // hysteresis band → partnerSpeaking flips true (the call strip lights their
  // ring). Audio playback is independent (the <audio> element), so degradation
  // when Web Audio is unavailable is silent and honest: the call still works,
  // just without the "who's talking" hint.

  /** Start polling the remote stream for speech. No-op if Web Audio is
   *  unavailable (rare in WebView2). Idempotent via stopSpeakingDetect. */
  private startSpeakingDetect(stream: MediaStream): void {
    this.stopSpeakingDetect();
    const Ctx: typeof AudioContext | undefined =
      window.AudioContext ?? (window as unknown as { webkitAudioContext?: typeof AudioContext }).webkitAudioContext;
    if (!Ctx || typeof AnalyserNode === "undefined") return;
    let ctx: AudioContext;
    let analyser: AnalyserNode;
    try {
      ctx = new Ctx();
      const src = ctx.createMediaStreamSource(stream);
      analyser = ctx.createAnalyser();
      analyser.fftSize = 256; // 128 time-domain bins — cheap, plenty for RMS
      src.connect(analyser);
      // The context may start suspended under autoplay policy; the "Entrar em
      // call" click gives sticky activation, so try to resume (best-effort).
      void ctx.resume().catch(() => {});
    } catch {
      return; // degrade — no ring, audio still plays via the <audio> element
    }
    this.audioCtx = ctx;
    this.analyser = analyser;

    const buf = new Uint8Array(analyser.fftSize);
    let speaking = false;
    // Hysteresis so the ring doesn't flicker at the threshold: a higher bar to
    // turn ON than to turn OFF. Tuned for typical speech over a PTT channel.
    const TURN_ON = 0.04;
    const TURN_OFF = 0.025;
    const tick = () => {
      const an = this.analyser;
      if (!an) return;
      an.getByteTimeDomainData(buf);
      let sum = 0;
      for (let i = 0; i < buf.length; i++) {
        const v = (buf[i] - 128) / 128; // centered to ~[-1, 1]
        sum += v * v;
      }
      const rms = Math.sqrt(sum / buf.length);
      if (!speaking && rms > TURN_ON) {
        speaking = true;
        this.set({ partnerSpeaking: true });
      } else if (speaking && rms < TURN_OFF) {
        speaking = false;
        this.set({ partnerSpeaking: false });
      }
    };
    this.speakTimer = window.setInterval(tick, 100);
  }

  /** Stop the poll + release the AudioContext. Called on teardown and before a
   *  fresh analysis starts (idempotent). */
  private stopSpeakingDetect(): void {
    if (this.speakTimer != null) {
      clearInterval(this.speakTimer);
      this.speakTimer = null;
    }
    if (this.audioCtx) {
      void this.audioCtx.close().catch(() => {});
      this.audioCtx = null;
    }
    this.analyser = null;
  }

  // --- timers ---------------------------------------------------------------

  /** Offerer cadence: while `reconnecting`, re-offer every RECONNECT_RETRY_MS.
   *  Safe to call from any state — it defers if not reconnecting, and is
   *  idempotent (clears any prior timer first). The responder also uses it to
   *  nudge itself out of a stuck wait, though its connect() path is a no-op
   *  (only the offerer creates offers). */
  private scheduleRetry(): void {
    if (!this.running) return;
    this.clearRetry();
    this.retryTimer = window.setTimeout(() => {
      this.retryTimer = null;
      if (!this.running) return;
      if (this.state.status !== "reconnecting") return;
      if (this.pc) return;
      if (socket.getStatus() !== "open") return;
      if (!this.localStream) {
        this.set({ status: "needs_permission" });
        return;
      }
      if (this.isOfferer) this.connect();
      // Responder: nothing to send; just re-arm the wait. (An inbound offer is
      // the only thing that lifts a responder out of reconnecting.)
      else this.scheduleRetry();
    }, RECONNECT_RETRY_MS);
  }

  private clearRetry(): void {
    if (this.retryTimer != null) {
      clearTimeout(this.retryTimer);
      this.retryTimer = null;
    }
  }

  /** Watchdog: if a `connecting` attempt doesn't reach `connected` within
   *  CONNECTING_TIMEOUT_MS, abandon it and go back to reconnecting (offerer
   *  re-offers). Covers an offer sent to an offline/late partner. */
  private startConnectingTimer(): void {
    this.clearConnectingTimer();
    this.connectingTimer = window.setTimeout(() => {
      this.connectingTimer = null;
      if (!this.running) return;
      if (this.state.status !== "connecting") return;
      this.teardownPC();
      this.set({ status: "reconnecting", partnerSpeaking: false });
      this.scheduleRetry();
    }, CONNECTING_TIMEOUT_MS);
  }

  private clearConnectingTimer(): void {
    if (this.connectingTimer != null) {
      clearTimeout(this.connectingTimer);
      this.connectingTimer = null;
    }
  }

  // --- teardown -------------------------------------------------------------

  /** Tear down the peer connection + remote analysis + PTT poll, but KEEP the
   *  local mic (muted) so a reconnect doesn't re-prompt for getUserMedia. */
  private teardownPC(): void {
    this.stopPTT();
    this.stopSpeakingDetect();
    this.pc?.close();
    this.pc = null;
    this.remoteStream = null;
    this.audioEls.forEach((el) => (el.srcObject = null));
  }

  /** Fully release the local mic (call on stopAlwaysOn — no partner, no need to
   *  hold the device open). Sets localStream null so the next startAlwaysOn
   *  re-prompts via the optimistic getUserMedia path. */
  private releaseMic(): void {
    this.localStream?.getTracks().forEach((t) => t.stop());
    this.localStream = null;
  }

  private set(patch: Partial<VoiceState>): void {
    this.state = { ...this.state, ...patch };
    this.listeners.forEach((l) => l(this.state));
  }
}

export const voice = new VoiceManager();
