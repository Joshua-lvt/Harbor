/**
 * Supabase Realtime Broadcast transport for ephemeral voice signaling.
 *
 * The caller must provide a short-lived JWT minted for this exact pair room.
 * The public anon key identifies the Supabase project; it is not authorization.
 * This transport never stores SDP/ICE and never falls back to a public channel.
 */
import {
  createClient,
  type RealtimeChannel,
  type SupabaseClient,
} from "@supabase/supabase-js";
import { SUPABASE_ANON_KEY, SUPABASE_URL } from "../../lib/supabase";
import { isRoomParticipant } from "./room";
import {
  SIGNALING_EVENT,
  parseSignalEnvelope,
  type SignalEnvelope,
  type SignalingStatus,
  type SignalingTransport,
} from "./types";

const TOPIC_PREFIX = "harbor:voice:";
const MAX_SEEN_MESSAGES = 512;

export interface SupabaseBroadcastOptions {
  roomId: string;
  localDeviceId: string;
  partnerDeviceId: string;
  /** Short-lived JWT authorized for this room; never use the anon key here. */
  accessToken: string;
  client?: SupabaseClient;
}

let defaultClient: SupabaseClient | null = null;

function getDefaultClient(): SupabaseClient {
  if (!defaultClient) {
    defaultClient = createClient(SUPABASE_URL, SUPABASE_ANON_KEY, {
      auth: {
        persistSession: false,
        autoRefreshToken: false,
        detectSessionInUrl: false,
      },
    });
  }
  return defaultClient;
}

export class SupabaseBroadcastTransport implements SignalingTransport {
  readonly name = "supabase-broadcast";
  private readonly client: SupabaseClient;
  private readonly listeners = new Set<(message: SignalEnvelope) => void>();
  private readonly statusListeners = new Set<(status: SignalingStatus) => void>();
  private readonly seen = new Set<string>();
  private channel: RealtimeChannel | null = null;
  private status: SignalingStatus = "closed";

  constructor(private readonly options: SupabaseBroadcastOptions) {
    if (!options.accessToken) throw new Error("Broadcast requer um token de mídia");
    if (!isRoomParticipant(options.roomId, options.localDeviceId)) {
      throw new Error("o dispositivo local não pertence à sala de mídia");
    }
    if (!isRoomParticipant(options.roomId, options.partnerDeviceId)) {
      throw new Error("o parceiro não pertence à sala de mídia");
    }
    if (options.localDeviceId === options.partnerDeviceId) {
      throw new Error("uma sala de mídia requer dois dispositivos");
    }
    this.client = options.client ?? getDefaultClient();
  }

  async connect(): Promise<void> {
    if (this.channel && (this.status === "open" || this.status === "connecting")) return;
    this.setStatus("connecting");
    await this.client.realtime.setAuth(this.options.accessToken);
    const topic = `${TOPIC_PREFIX}${this.options.roomId}`;
    const channel = this.client.channel(topic, { config: { private: true } });
    this.channel = channel;
    channel.on("broadcast", { event: SIGNALING_EVENT }, (payload) => {
      this.handlePayload(payload.payload);
    });

    await new Promise<void>((resolve, reject) => {
      let settled = false;
      channel.subscribe((status, error) => {
        if (status === "SUBSCRIBED") {
          this.setStatus("open");
          if (!settled) {
            settled = true;
            resolve();
          }
        } else if (status === "CHANNEL_ERROR" || status === "TIMED_OUT") {
          this.setStatus("error");
          if (!settled) {
            settled = true;
            reject(error ?? new Error(`Broadcast subscription failed: ${status}`));
          }
        } else if (status === "CLOSED") {
          this.setStatus("closed");
        }
      });
    });
  }

  async publish(message: SignalEnvelope): Promise<void> {
    if (message.room_id !== this.options.roomId || message.sender_id !== this.options.localDeviceId) {
      throw new Error("Broadcast message does not belong to this transport");
    }
    if (!this.channel || this.status !== "open") throw new Error("Broadcast is not connected");
    const result = await this.channel.send({
      type: "broadcast",
      event: SIGNALING_EVENT,
      payload: message,
    });
    if (result !== "ok") throw new Error(`Broadcast publish failed: ${result}`);
  }

  /** Replace the expiring room JWT and rejoin so the new token is authenticated. */
  async updateAccessToken(accessToken: string): Promise<void> {
    if (!accessToken) throw new Error("Broadcast requer um token de mídia");
    const channel = this.channel;
    this.channel = null;
    this.options.accessToken = accessToken;
    this.seen.clear();
    if (channel) await this.client.removeChannel(channel);
    this.setStatus("closed");
    // A previous join may have failed and left no channel; retrying the token
    // still needs to establish the private subscription, not merely update state.
    await this.connect();
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
    const channel = this.channel;
    this.channel = null;
    this.seen.clear();
    if (channel) await this.client.removeChannel(channel);
    this.setStatus("closed");
  }

  private handlePayload(value: unknown): void {
    const message = parseSignalEnvelope(value);
    if (!message) return;
    if (message.room_id !== this.options.roomId) return;
    if (message.sender_id !== this.options.partnerDeviceId) return;
    const key = `${message.sender_id}:${message.seq}`;
    if (this.seen.has(key)) return;
    this.seen.add(key);
    if (this.seen.size > MAX_SEEN_MESSAGES) {
      const oldest = this.seen.values().next().value;
      if (typeof oldest === "string") this.seen.delete(oldest);
    }
    this.listeners.forEach((listener) => {
      try {
        listener(message);
      } catch {
        // A subscriber cannot break the Realtime channel.
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
        // Status observers are isolated from transport failures.
      }
    });
  }
}
