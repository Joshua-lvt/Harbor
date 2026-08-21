/**
 * Versioned signaling contract shared by relay and Supabase transports.
 *
 * Signaling is control-plane data only: SDP and ICE payloads are opaque to the
 * transport, never persisted, and delivered only to the other member of a
 * two-device room. `seq` makes duplicate/out-of-order Broadcast deliveries
 * observable without asking either transport to interpret SDP.
 */

export const SIGNALING_PROTOCOL_VERSION = 1 as const;
export const SIGNALING_EVENT = "harbor.voice.signal" as const;
/** Keep signaling frames comfortably below relay and Realtime limits. */
export const MAX_SIGNALING_BYTES = 256 * 1024;

export type SignalKind = "offer" | "answer" | "ice";

export interface SignalEnvelope {
  v: typeof SIGNALING_PROTOCOL_VERSION;
  room_id: string;
  sender_id: string;
  seq: number;
  kind: SignalKind;
  data: unknown;
  sent_at: number;
}

export type SignalingStatus = "closed" | "connecting" | "open" | "error";

export interface SignalingTransport {
  readonly name: string;
  connect(): Promise<void>;
  publish(message: SignalEnvelope): Promise<void>;
  onMessage(handler: (message: SignalEnvelope) => void): () => void;
  onStatus(handler: (status: SignalingStatus) => void): () => void;
  close(): Promise<void>;
}

export function signalByteLength(message: SignalEnvelope): number {
  return new TextEncoder().encode(JSON.stringify(message)).byteLength;
}

/**
 * Validate an inbound envelope before it reaches the WebRTC engine. Treat all
 * received data as hostile: Broadcast payloads and relay payloads are both
 * network input.
 */
export function parseSignalEnvelope(value: unknown): SignalEnvelope | null {
  if (!value || typeof value !== "object") return null;
  const m = value as Record<string, unknown>;
  if (m.v !== SIGNALING_PROTOCOL_VERSION) return null;
  if (typeof m.room_id !== "string" || !m.room_id || m.room_id.length > 256) return null;
  if (typeof m.sender_id !== "string" || !m.sender_id || m.sender_id.length > 256) return null;
  if (!Number.isSafeInteger(m.seq) || (m.seq as number) < 1) return null;
  if (m.kind !== "offer" && m.kind !== "answer" && m.kind !== "ice") return null;
  if (!Number.isFinite(m.sent_at) || (m.sent_at as number) <= 0) return null;
  if (m.data === undefined) return null;

  const message = value as SignalEnvelope;
  try {
    if (signalByteLength(message) > MAX_SIGNALING_BYTES) return null;
  } catch {
    return null;
  }
  return message;
}

export function makeSignalEnvelope(args: {
  roomId: string;
  senderId: string;
  seq: number;
  kind: SignalKind;
  data: unknown;
  sentAt?: number;
}): SignalEnvelope {
  const message: SignalEnvelope = {
    v: SIGNALING_PROTOCOL_VERSION,
    room_id: args.roomId,
    sender_id: args.senderId,
    seq: args.seq,
    kind: args.kind,
    data: args.data,
    sent_at: args.sentAt ?? Date.now(),
  };
  if (!parseSignalEnvelope(message)) throw new Error("invalid signaling envelope");
  return message;
}
