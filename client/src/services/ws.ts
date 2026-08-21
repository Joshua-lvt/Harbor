/**
 * WebSocket client to the Harbor relay.
 *
 * The main window owns this singleton for the app lifetime. Auto-reconnect
 * with exponential backoff (1s → 2 → 4 → 8 → 15s cap). Heartbeats every 60s;
 * the relay times dead sockets out after ~180s. Re-announces the last known
 * presence on every (re)connect so a dropped link doesn't reset "away".
 *
 * - Idempotent connect() — safe to call from any screen/route.
 * - New envelope types (activity, voice_signal) flow through the same
 *   send()/onEvent() API alongside chat/presence/typing/last_seen.
 */
import type { ClientEvent, ServerEvent } from "../lib/types";

type Handler = (e: ServerEvent) => void;
type StatusHandler = (status: "connecting" | "open" | "closed") => void;

function toWsUrl(relayUrl: string, deviceId: string, deviceSecret: string): string {
  let base = relayUrl.replace(/[?#].*$/, "").replace(/\/$/, "");
  let wsBase: string;
  if (base.startsWith("wss://")) wsBase = base;
  else if (base.startsWith("ws://")) wsBase = base;
  else if (base.startsWith("https://")) wsBase = "wss://" + base.slice("https://".length);
  else if (base.startsWith("http://")) wsBase = "ws://" + base.slice("http://".length);
  else wsBase = "ws://" + base;
  const path = wsBase.endsWith("/ws") ? wsBase : wsBase + "/ws";
  return `${path}?device_id=${encodeURIComponent(deviceId)}&secret=${encodeURIComponent(deviceSecret)}`;
}

class HarborSocket {
  private ws: WebSocket | null = null;
  private url = "";
  private handlers = new Set<Handler>();
  private statusHandlers = new Set<StatusHandler>();
  private reconnectAttempt = 0;
  private heartbeatTimer: number | null = null;
  private manualClose = false;
  private status: "connecting" | "open" | "closed" = "closed";
  // Last announced presence. Re-announced on every (re)connect so a dropped
  // connection doesn't reset "away" to "online" in the partner's view. The
  // reconnect handshake would otherwise re-assert online against an OS-idle
  // device that is genuinely away.
  private presence: "online" | "away" = "online";

  onEvent(h: Handler): () => void {
    this.handlers.add(h);
    return () => this.handlers.delete(h);
  }
  onStatus(h: StatusHandler): () => void {
    this.statusHandlers.add(h);
    h(this.status);
    return () => this.statusHandlers.delete(h);
  }
  getStatus() {
    return this.status;
  }

  /** Connect to the relay. Idempotent: if already open/connecting, no-op. */
  connect(relayUrl: string, deviceId: string, deviceSecret: string): void {
    if (this.status === "open" || this.status === "connecting") return;
    this.url = toWsUrl(relayUrl, deviceId, deviceSecret);
    this.manualClose = false;
    this.open();
  }
  private open() {
    if (!this.url) return;
    this.setStatus("connecting");
    this.ws = new WebSocket(this.url);
    this.ws.onopen = () => {
      this.reconnectAttempt = 0;
      this.setStatus("open");
      this.startHeartbeat();
      // Re-assert the last known presence (online or away), not a hardcoded
      // online — see `presence` field docs. Heartbeats alone no longer flip
      // the server's stored presence, so it must be re-sent here.
      this.send({ type: "presence", state: this.presence });
    };
    this.ws.onmessage = (ev) => {
      try {
        const payload = JSON.parse(ev.data) as ServerEvent;
        // Dispatch in-process to subscribers (Home, Chat, presence, notify).
        this.handlers.forEach((h) => {
          try {
            h(payload);
          } catch {
            // a bad subscriber must not break others
          }
        });
      } catch {
        // ignore malformed server frame
      }
    };
    this.ws.onclose = () => {
      this.stopHeartbeat();
      this.setStatus("closed");
      if (!this.manualClose) this.scheduleReconnect();
    };
    this.ws.onerror = () => {
      // onclose will follow and handle reconnect.
    };
  }

  send(ev: ClientEvent): void {
    if (this.ws && this.ws.readyState === WebSocket.OPEN) {
      this.ws.send(JSON.stringify(ev));
    }
  }

  /** Update + send a presence change. Tracked so reconnects re-assert it. */
  setPresence(state: "online" | "away"): void {
    this.presence = state;
    this.send({ type: "presence", state });
  }

  /** Tear down permanently (used on logout/unpair, not on screen change). */
  close(): void {
    this.manualClose = true;
    this.stopHeartbeat();
    this.ws?.close();
    this.ws = null;
    this.setStatus("closed");
  }

  private startHeartbeat() {
    this.stopHeartbeat();
    this.heartbeatTimer = window.setInterval(() => {
      this.send({ type: "heartbeat" });
    }, 60_000);
  }
  private stopHeartbeat() {
    if (this.heartbeatTimer != null) {
      clearInterval(this.heartbeatTimer);
      this.heartbeatTimer = null;
    }
  }
  private scheduleReconnect() {
    const delay = Math.min(1000 * 2 ** this.reconnectAttempt, 15_000);
    this.reconnectAttempt += 1;
    window.setTimeout(() => {
      if (!this.manualClose) this.open();
    }, delay);
  }
  private setStatus(s: "connecting" | "open" | "closed") {
    this.status = s;
    this.statusHandlers.forEach((h) => h(s));
  }
}

export const socket = new HarborSocket();
