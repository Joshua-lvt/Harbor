/**
 * Compatibility signaling transport backed by the existing Harbor relay.
 *
 * The relay predates room-aware envelopes, so this adapter supplies the local
 * room id and a local receive sequence while preserving the wire-compatible
 * `voice_signal` event. It is intentionally kept as a fallback during the
 * Supabase Realtime rollout.
 */
import type { ServerEvent } from "../../lib/types";
import { socket } from "../ws";
import {
  makeSignalEnvelope,
  parseSignalEnvelope,
  type SignalEnvelope,
  type SignalingStatus,
  type SignalingTransport,
} from "./types";

export interface RelaySocketLike {
  getStatus(): "connecting" | "open" | "closed";
  onEvent(handler: (event: ServerEvent) => void): () => void;
  onStatus(handler: (status: "connecting" | "open" | "closed") => void): () => void;
  send(event: { type: "voice_signal"; kind: SignalEnvelope["kind"]; data: unknown }): void;
}

export class RelaySignalingTransport implements SignalingTransport {
  readonly name = "relay";
  private readonly listeners = new Set<(message: SignalEnvelope) => void>();
  private readonly statusListeners = new Set<(status: SignalingStatus) => void>();
  private offEvent: (() => void) | null = null;
  private offStatus: (() => void) | null = null;
  private receiveSeq = 0;
  private status: SignalingStatus = "closed";

  constructor(
    private readonly roomId: string,
    private readonly localDeviceId: string,
    private readonly partnerDeviceId: string,
    private readonly relay: RelaySocketLike = socket,
  ) {}

  async connect(): Promise<void> {
    if (this.offEvent) return;
    this.offEvent = this.relay.onEvent((event) => this.onEvent(event));
    this.offStatus = this.relay.onStatus((status) => {
      this.setStatus(status === "open" ? "open" : status === "connecting" ? "connecting" : "closed");
    });
    this.setStatus(this.relay.getStatus() === "open" ? "open" : "connecting");
  }

  async publish(message: SignalEnvelope): Promise<void> {
    if (message.room_id !== this.roomId || message.sender_id !== this.localDeviceId) {
      throw new Error("relay signaling message does not belong to this transport");
    }
    if (this.relay.getStatus() !== "open") throw new Error("relay signaling is not connected");
    this.relay.send({ type: "voice_signal", kind: message.kind, data: message.data });
  }

  onMessage(handler: (message: SignalEnvelope) => void): () => void {
    this.listeners.add(handler);
    return () => this.listeners.delete(handler);
  }

  onStatus(handler: (status: SignalingStatus) => void): () => void {
    this.statusListeners.add(handler);
    handler(this.status);
    return () => this.statusListeners.delete(handler);
  }

  async close(): Promise<void> {
    this.offEvent?.();
    this.offStatus?.();
    this.offEvent = null;
    this.offStatus = null;
    this.setStatus("closed");
  }

  private onEvent(event: ServerEvent): void {
    if (event.type !== "voice_signal" || event.device_id !== this.partnerDeviceId) return;
    const message = parseSignalEnvelope(
      makeSignalEnvelope({
        roomId: this.roomId,
        senderId: event.device_id,
        seq: ++this.receiveSeq,
        kind: event.kind,
        data: event.data,
        // Relay timestamps are epoch seconds; the common contract uses ms.
        sentAt: event.ts * 1000,
      }),
    );
    if (!message) return;
    this.listeners.forEach((listener) => {
      try {
        listener(message);
      } catch {
        // One consumer must not break signaling for the other subscribers.
      }
    });
  }

  private setStatus(status: SignalingStatus): void {
    if (this.status === status) return;
    this.status = status;
    this.statusListeners.forEach((listener) => {
      try {
        listener(status);
      } catch {
        // Status observers are isolated from the transport.
      }
    });
  }
}
