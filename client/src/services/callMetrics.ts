/**
 * callMetrics — lightweight, local-only diagnostics for the WebRTC call.
 *
 * The user wants to know how many calls use P2P (host/srflx) vs TURN (relay),
 * how long they last, and how often ICE fails — to decide whether a second TURN
 * provider is ever needed. Everything is recorded LOCALLY (localStorage), never
 * sent to Cloudflare/Supabase (zero network cost). A small readout is exposed
 * via `getSummary()` for a debug view.
 *
 * Transport classification (from the selected ICE candidate pair):
 *   host  → direct P2P on the LAN
 *   srflx → P2P via STUN (public IP, still direct)
 *   relay → TURN relay (P2P impossible; media goes through ExpressTURN)
 *
 * Both `host` and `srflx` count as "P2P" for the summary; only `relay` counts
 * as TURN.
 */
import { asMs } from "../lib/localDb";

const KEY = "harbor:call-metrics";

export type Transport = "host" | "srflx" | "relay" | "unknown";

export interface CallSession {
  transport: Transport;
  /** Duration in ms. */
  durationMs: number;
  /** True if the call ended because the connection failed (vs a clean stop). */
  failed: boolean;
  /** ICE reconnection count during the call. */
  reconnects: number;
  ts: number;
}

export interface CallSummary {
  totalCalls: number;
  p2pCalls: number;
  turnCalls: number;
  unknownCalls: number;
  totalDurationMs: number;
  iceFailures: number;
  reconnects: number;
  /** Most recent transport, for a live "P2P / TURN" badge. */
  lastTransport: Transport;
}

interface Store {
  sessions: CallSession[];
}

const MAX_SESSIONS = 500;

function load(): Store {
  try {
    const raw = localStorage.getItem(KEY);
    if (!raw) return { sessions: [] };
    const parsed = JSON.parse(raw);
    if (parsed && Array.isArray(parsed.sessions)) return parsed as Store;
    return { sessions: [] };
  } catch {
    return { sessions: [] };
  }
}

function save(s: Store): void {
  try {
    localStorage.setItem(KEY, JSON.stringify(s));
  } catch {
    // quota / unavailable — the in-memory session still works
  }
}

/** Record a completed call session. */
export function recordCall(session: Omit<CallSession, "ts">): void {
  const store = load();
  store.sessions.push({ ...session, ts: Date.now() });
  if (store.sessions.length > MAX_SESSIONS) {
    store.sessions = store.sessions.slice(-MAX_SESSIONS);
  }
  save(store);
}

/** Aggregate the recorded sessions into a summary. */
export function getSummary(): CallSummary {
  const { sessions } = load();
  const summary: CallSummary = {
    totalCalls: sessions.length,
    p2pCalls: 0,
    turnCalls: 0,
    unknownCalls: 0,
    totalDurationMs: 0,
    iceFailures: 0,
    reconnects: 0,
    lastTransport: "unknown",
  };
  for (const s of sessions) {
    summary.totalDurationMs += s.durationMs;
    summary.iceFailures += s.failed ? 1 : 0;
    summary.reconnects += s.reconnects;
    if (s.transport === "relay") summary.turnCalls += 1;
    else if (s.transport === "host" || s.transport === "srflx") summary.p2pCalls += 1;
    else summary.unknownCalls += 1;
  }
  if (sessions.length) summary.lastTransport = sessions[sessions.length - 1].transport;
  return summary;
}

/** Classify a candidate type string from getStats into our Transport union. */
export function classifyTransport(candidateType: string | undefined): Transport {
  if (candidateType === "host") return "host";
  if (candidateType === "srflx") return "srflx";
  if (candidateType === "relay") return "relay";
  return "unknown";
}

/** Human label for a transport, in pt-BR. */
export function transportLabel(t: Transport): string {
  if (t === "host") return "P2P direto";
  if (t === "srflx") return "P2P via STUN";
  if (t === "relay") return "TURN (relay)";
  return "desconhecido";
}

/** Re-export asMs so voice.ts can timestamp sessions without importing localDb
 *  directly (keeps the metrics module self-contained). */
export { asMs };
