/**
 * Tauri/WebView side of the Linux native media bridge.
 *
 * The bridge deliberately does not replace the existing JS WebRTC engine yet:
 * capabilities.native_webrtc is false until the webrtc-rs session exists. It
 * does, however, make the capture lifecycle and signaling path executable now,
 * with the same offer/answer/ICE envelope that the relay already transports.
 */
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export const MEDIA_PROTOCOL_VERSION = 1 as const;
export const MEDIA_STATE_EVENT = "media_state" as const;
export const MEDIA_SIGNAL_EVENT = "media_signal" as const;
export const MEDIA_AUDIO_EVENT = "media_audio" as const;
export const MEDIA_VIDEO_EVENT = "media_video" as const;

export type NativeSignalKind = "offer" | "answer" | "ice";

export interface NativeMediaSignal {
  v: typeof MEDIA_PROTOCOL_VERSION;
  room_id: string;
  sender_id: string;
  seq: number;
  kind: NativeSignalKind;
  data: unknown;
  sent_at: number;
}

export interface NativeMediaCapabilities {
  protocol_version: number;
  backend: string;
  audio_capture: boolean;
  screen_capture: boolean;
  screen_backend: string;
  native_webrtc: boolean;
}

export interface NativeMediaState {
  state: string;
  backend: string;
  detail: string | null;
}

export interface NativeMediaFrame {
  timestamp_ns: number;
  data_base64: string;
  dropped_before_emit: number;
}

export interface NativeMediaStartOptions {
  room_id: string;
  device_id: string;
  partner_id: string;
  screen?: boolean;
}

/** Minimal shape of the existing legacy relay socket. */
export interface VoiceSignalingTransport {
  send(message: { type: "voice_signal"; kind: NativeSignalKind; data: unknown }): void;
  onEvent(handler: (event: unknown) => void): () => void;
}

export interface NativeSignalingContext {
  roomId: string;
  localDeviceId: string;
  partnerDeviceId: string;
}

function isVoiceSignalEvent(event: unknown): event is {
  type: "voice_signal";
  kind: NativeSignalKind;
  data: unknown;
} {
  if (!event || typeof event !== "object") return false;
  const value = event as Record<string, unknown>;
  return (
    value.type === "voice_signal" &&
    (value.kind === "offer" || value.kind === "answer" || value.kind === "ice")
  );
}

export function isNativeSignalFromPartner(
  signal: Pick<NativeMediaSignal, "room_id" | "sender_id">,
  context: NativeSignalingContext,
): boolean {
  return (
    signal.room_id === context.roomId &&
    signal.sender_id === context.partnerDeviceId &&
    signal.sender_id !== context.localDeviceId
  );
}

export class NativeLinuxMediaBridge {
  readonly name = "native-linux-media";

  /** Tauri v2 exposes this marker only inside the desktop WebView. */
  isAvailable(): boolean {
    return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
  }

  capabilities(): Promise<NativeMediaCapabilities> {
    return invoke<NativeMediaCapabilities>("media_capabilities");
  }

  start(options: NativeMediaStartOptions): Promise<NativeMediaCapabilities> {
    return invoke<NativeMediaCapabilities>("media_start", { request: options });
  }

  stop(): Promise<void> {
    return invoke("media_stop");
  }

  setPtt(active: boolean): Promise<void> {
    return invoke("media_set_ptt", { active });
  }

  receiveSignal(signal: NativeMediaSignal): Promise<void> {
    return invoke("media_receive_signal", { signal });
  }

  onState(handler: (state: NativeMediaState) => void): Promise<UnlistenFn> {
    return listen<NativeMediaState>(MEDIA_STATE_EVENT, (event) => handler(event.payload));
  }

  onSignal(handler: (signal: NativeMediaSignal) => void): Promise<UnlistenFn> {
    return listen<NativeMediaSignal>(MEDIA_SIGNAL_EVENT, (event) => handler(event.payload));
  }

  onAudio(handler: (frame: NativeMediaFrame) => void): Promise<UnlistenFn> {
    return listen<NativeMediaFrame>(MEDIA_AUDIO_EVENT, (event) => handler(event.payload));
  }

  onVideo(handler: (frame: NativeMediaFrame) => void): Promise<UnlistenFn> {
    return listen<NativeMediaFrame>(MEDIA_VIDEO_EVENT, (event) => handler(event.payload));
  }

  /**
   * Connect native signaling to the existing relay socket. Inbound relay
   * envelopes are handed to native; native envelopes are sent back through the
   * relay. The returned async cleanup removes both listeners.
   */
  async attachSignaling(
    transport: VoiceSignalingTransport,
    context: NativeSignalingContext,
  ): Promise<() => void> {
    const unlistenNative = await this.onSignal((signal) => {
      transport.send({ type: "voice_signal", kind: signal.kind, data: signal.data });
    });
    const offRelay = transport.onEvent((event) => {
      if (!isVoiceSignalEvent(event)) return;
      const value = event as Record<string, unknown>;
      const senderId = typeof value.device_id === "string" ? value.device_id : "";
      const signal: NativeMediaSignal = {
        v: MEDIA_PROTOCOL_VERSION,
        room_id: context.roomId,
        sender_id: senderId,
        seq: typeof value.seq === "number" ? value.seq : 0,
        kind: event.kind,
        data: event.data,
        sent_at: typeof value.ts === "number" ? value.ts : Date.now(),
      };
      // The sender is supplied by the relay; reject malformed or unrelated
      // events before they reach Rust. Rust repeats this check against the
      // active media session because the WebView boundary is untrusted too.
      if (senderId && isNativeSignalFromPartner(signal, context)) {
        void this.receiveSignal(signal).catch(() => {});
      }
    });
    return () => {
      unlistenNative();
      offRelay();
    };
  }
}

/** Native WebRTC is opt-in only after the Rust peer session is available. */
export function shouldUseNativeLinuxMedia(
  bridge: NativeLinuxMediaBridge,
  capabilities: NativeMediaCapabilities,
): boolean {
  return bridge.isAvailable() && capabilities.native_webrtc;
}
