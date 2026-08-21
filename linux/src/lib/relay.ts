/**
 * HTTP client for the relay's REST endpoints.
 *
 * `relayUrl` is a WebSocket URL (ws:// or wss://); we derive the matching http(s)
 * base for fetch by swapping the scheme. All requests carry the device_secret
 * in the body (the relay uses it to authenticate mutations on behalf of a
 * device). Keep this on TLS in any non-local deployment.
 */
import type { Identity } from "./types";

export interface RegisterResponse {
  pairing_code: string;
  device_secret: string;
}

export interface PairResponse {
  partner_device_id: string;
  partner_name: string | null;
  /** The partner's X25519 public key (base64) — null if they haven't published
   *  one yet (client falls back to plaintext, visibly marked insecure). */
  partner_public_key: string | null;
  /** The partner's profile photo (base64 JPEG data URL) — null if they haven't
   *  set one yet (client falls back to the shark mascot avatar). */
  partner_avatar: string | null;
}

export interface PartnerInfo {
  partner_device_id: string;
  partner_name: string | null;
  presence: string;
  last_seen: number | null;
  /** The partner's X25519 public key (base64) — retrieved on cold start by the
   *  passive partner (who wasn't the one to call /pair). */
  partner_public_key: string | null;
  /** The partner's profile photo (base64 JPEG data URL) — retrieved on cold
   *  start by the passive partner. */
  partner_avatar: string | null;
}

/** The caller's own state — used at cold start to recover a fresh code after
 *  the partner unpaired us while we were offline. */
export interface MeInfo {
  pairing_code: string | null;
  partner_id: string | null;
  display_name: string | null;
}

/** Result of breaking the pairing — our newly reissued code (pair again now). */
export interface UnpairResponse {
  ok: boolean;
  pairing_code: string;
}

function httpBase(wsUrl: string): string {
  if (wsUrl.startsWith("wss://")) return "https://" + wsUrl.slice("wss://".length);
  if (wsUrl.startsWith("ws://")) return "http://" + wsUrl.slice("ws://".length);
  // Already http(s) or something else: use as-is but strip any trailing slash.
  return wsUrl.replace(/\/$/, "");
}

async function relayFetch(path: string, init: RequestInit & { relayUrl: string }): Promise<Response> {
  const base = httpBase(init.relayUrl).replace(/\/$/, "");
  const res = await fetch(base + path, {
    ...init,
    headers: { "Content-Type": "application/json", ...(init.headers ?? {}) },
  });
  if (!res.ok) {
    let detail = res.statusText;
    try {
      const body = await res.json();
      detail = body.detail ?? detail;
    } catch {
      // ignore parse error
    }
    throw new Error(`relay ${path}: ${res.status} ${detail}`);
  }
  return res;
}

/** Register this device (idempotent for the same device_id in the relay).
 *  `publicKey` (base64 X25519) is published so a future partner can seal
 *  messages to us; optional so pre-E2E clients/scripts keep working. `avatar`
 *  (base64 JPEG data URL) is published so a future partner can show our photo.
 *  Both `undefined`/`null` are omitted by JSON.stringify, keeping the wire
 *  byte-identical when absent. */
export async function register(
  relayUrl: string,
  deviceId: string,
  publicKey?: string | null,
  avatar?: string | null,
): Promise<RegisterResponse> {
  const res = await relayFetch("/register", {
    relayUrl,
    method: "POST",
    body: JSON.stringify({
      device_id: deviceId,
      public_key: publicKey ?? undefined,
      avatar: avatar ?? undefined,
    }),
  });
  return res.json();
}

/** Link to a partner by pasting their pairing code. */
export async function pair(id: Identity, partnerCode: string): Promise<PairResponse> {
  const res = await relayFetch("/pair", {
    relayUrl: id.relay_url,
    method: "POST",
    body: JSON.stringify({
      device_id: id.device_id,
      device_secret: id.device_secret,
      partner_code: partnerCode,
    }),
  });
  return res.json();
}

/** Set the user's own display name so the partner sees it. `publicKey` is the
 *  upgrade-publish path: an already-paired install that predates E2E publishes
 *  its key here (avoids re-/register, which would re-issue the device_secret
 *  and force a WS re-auth). `avatar` (base64 JPEG data URL) follows the same
 *  contract — pass "" to clear the stored photo, null/undefined to leave it
 *  untouched. All optional. */
export async function setProfile(
  id: Identity,
  /** The partner-facing display name. `null` lets the cold-start upgrade path
   *  (App.tsx) publish a public key for a pre-E2E install without inventing a
   *  name — the relay stores NULL, preserving the no-name state. */
  displayName: string | null,
  publicKey?: string | null,
  avatar?: string | null,
): Promise<void> {
  await relayFetch("/profile", {
    relayUrl: id.relay_url,
    method: "POST",
    body: JSON.stringify({
      device_id: id.device_id,
      device_secret: id.device_secret,
      display_name: displayName,
      public_key: publicKey ?? undefined,
      avatar: avatar ?? undefined,
    }),
  });
}

/** How long a cached /partner result stays fresh. The partner's STATIC info
 *  (name, pubkey, avatar) changes rarely, and presence is refreshed live over
 *  the socket — so a short in-memory cache lets the 3 call sites (App cold
 *  start, HomeScreen mount, Chat mount) share ONE fetch per launch instead of
 *  three, without ever showing stale presence. */
const PARTNER_CACHE_TTL_MS = 30_000;

/** Fetch the partner's current presence + last_seen (used on cold start).
 *  Cached in memory for PARTNER_CACHE_TTL_MS keyed by device_id, so the
 *  redundant cold-start / Home / Chat fetches collapse into a single request. */
export async function getPartner(id: Identity): Promise<PartnerInfo> {
  const now = Date.now();
  const hit = partnerCache.get(id.device_id);
  if (hit && now - hit.at < PARTNER_CACHE_TTL_MS) return hit.value;
  const pending = partnerRequests.get(id.device_id);
  if (pending) return pending;
  const request = (async () => {
    const base = httpBase(id.relay_url).replace(/\/$/, "");
    const url = `${base}/partner?device_id=${encodeURIComponent(id.device_id)}&secret=${encodeURIComponent(id.device_secret)}`;
    const res = await fetch(url);
    if (!res.ok) throw new Error(`relay /partner: ${res.status}`);
    const value = (await res.json()) as PartnerInfo;
    partnerCache.set(id.device_id, { at: Date.now(), value });
    return value;
  })();
  partnerRequests.set(id.device_id, request);
  try {
    return await request;
  } finally {
    if (partnerRequests.get(id.device_id) === request) partnerRequests.delete(id.device_id);
  }
}

/** In-memory /partner cache (see getPartner). */
const partnerCache = new Map<string, { at: number; value: PartnerInfo }>();
const partnerRequests = new Map<string, Promise<PartnerInfo>>();

/** Drop the cached /partner result for a device. Called on unpair so a quick
 *  re-pair (or a post-unpair query within the 30s TTL) doesn't return the
 *  previous partner's stale data. */
export function clearPartnerCache(deviceId: string): void {
  partnerCache.delete(deviceId);
}

/** Fetch my own state (pairing code, partner link, display name) — used at
 *  cold start: if /partner 404s, the partner unpaired us while we were offline,
 *  so we resync our pairing_code from here and return to the pairing screen. */
export async function getMe(id: Identity): Promise<MeInfo> {
  const base = httpBase(id.relay_url).replace(/\/$/, "");
  const url = `${base}/me?device_id=${encodeURIComponent(id.device_id)}&secret=${encodeURIComponent(id.device_secret)}`;
  const res = await fetch(url);
  if (!res.ok) throw new Error(`relay /me: ${res.status}`);
  return res.json();
}

/** Break the current pairing (bilateral). Returns our freshly reissued pairing
 *  code so the client can immediately show it and pair with a new person.
 *  The relay pushes an `unpaired` event to the ex-partner over their live WS. */
export async function unpair(id: Identity): Promise<UnpairResponse> {
  const res = await relayFetch("/unpair", {
    relayUrl: id.relay_url,
    method: "POST",
    body: JSON.stringify({
      device_id: id.device_id,
      device_secret: id.device_secret,
    }),
  });
  return res.json();
}
