/** Obtain the short-lived Realtime JWT used by the private media channel. */
import { SUPABASE_ANON_KEY, SUPABASE_URL } from "../lib/supabase";
import { roomIdForPair } from "./signaling/room";

export interface MediaSessionRequest {
  deviceId: string;
  deviceSecret: string;
  partnerId: string;
}

export interface MediaSession {
  accessToken: string;
  roomId: string;
  mediaTopic: string;
  expiresAtMs: number;
}

export interface MediaSessionOptions {
  endpoint?: string;
  fetchImpl?: typeof fetch;
  now?: () => number;
}

export async function createMediaSession(
  request: MediaSessionRequest,
  options: MediaSessionOptions = {},
): Promise<MediaSession> {
  const roomId = roomIdForPair(request.deviceId, request.partnerId);
  const fetchImpl = options.fetchImpl ?? fetch;
  const now = options.now ?? (() => Date.now());
  const endpoint = options.endpoint ?? `${SUPABASE_URL}/functions/v1/media-session`;
  try {
    if (new URL(endpoint).protocol !== "https:") {
      throw new Error("media session endpoint must use HTTPS");
    }
  } catch (error) {
    if (error instanceof Error && error.message.includes("must use HTTPS")) throw error;
    throw new Error("media session endpoint must use HTTPS");
  }
  const response = await fetchImpl(endpoint, {
    method: "POST",
    headers: {
      apikey: SUPABASE_ANON_KEY,
      "Content-Type": "application/json",
    },
    // This is sent only over HTTPS to the Edge Function, which validates it via
    // the private Harbor authorization endpoint and never returns it.
    body: JSON.stringify({
      device_id: request.deviceId,
      device_secret: request.deviceSecret,
      partner_id: request.partnerId,
      room_id: roomId,
    }),
  });
  if (!response.ok) throw new Error(`media session failed (${response.status})`);
  const payload = (await response.json()) as Record<string, unknown>;
  const accessToken = typeof payload.access_token === "string" ? payload.access_token : "";
  const mediaTopic = typeof payload.media_topic === "string" ? payload.media_topic : "";
  const responseRoom = typeof payload.room_id === "string" ? payload.room_id : "";
  const expectedMediaTopic = `harbor:voice:${roomId}`;
  const expiry = typeof payload.expires_at === "number" ? payload.expires_at : NaN;
  const expiresAtMs = expiry < 10_000_000_000 ? expiry * 1000 : expiry;
  if (
    !accessToken ||
    mediaTopic !== expectedMediaTopic ||
    responseRoom !== roomId ||
    !Number.isFinite(expiresAtMs) ||
    expiresAtMs <= now() ||
    expiresAtMs - now() > 15 * 60 * 1000
  ) {
    throw new Error("media session response is invalid or expired");
  }
  return { accessToken, roomId, mediaTopic, expiresAtMs };
}
