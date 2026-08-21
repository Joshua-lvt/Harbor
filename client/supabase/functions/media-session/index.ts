import { json, jsonBody, options } from "../_shared/http.ts";
import { signMediaJwt } from "../_shared/jwt.ts";

interface AuthResult {
  authorized?: unknown;
  role?: unknown;
  partner_id?: unknown;
  pair_key?: unknown;
}

const MAX_TOKEN_TTL_SECONDS = 10 * 60;

function canonicalRoomId(deviceId: string, partnerId: string): string | null {
  if (!deviceId || !partnerId || deviceId === partnerId) return null;
  const [first, second] = [deviceId, partnerId].sort();
  return `pair:${encodeURIComponent(first)}:${encodeURIComponent(second)}`;
}

function topicForRoom(roomId: string): string {
  return `harbor:voice:${roomId}`;
}

function configuredTtl(): number {
  const raw = Number(Deno.env.get("MEDIA_TOKEN_TTL_SECONDS") ?? "300");
  if (!Number.isFinite(raw) || raw < 60) return 300;
  return Math.min(Math.floor(raw), MAX_TOKEN_TTL_SECONDS);
}

async function authorizeWithHarbor(
  deviceId: string,
  deviceSecret: string,
  roomId: string,
  partnerId: string,
): Promise<AuthResult | null> {
  const authUrl = Deno.env.get("HARBOR_MEDIA_AUTH_URL");
  const internalToken = Deno.env.get("HARBOR_MEDIA_AUTH_TOKEN");
  if (!authUrl || !internalToken) return null;

  try {
    const response = await fetch(authUrl, {
      method: "POST",
      headers: {
        Authorization: `Bearer ${internalToken}`,
        "Content-Type": "application/json",
      },
      body: JSON.stringify({
        device_id: deviceId,
        device_secret: deviceSecret,
        room_id: roomId,
        partner_id: partnerId,
      }),
    });
    if (!response.ok) return null;
    const value: unknown = await response.json();
    if (!value || typeof value !== "object" || Array.isArray(value)) return null;
    return value as AuthResult;
  } catch {
    return null;
  }
}

async function handle(request: Request): Promise<Response> {
  if (request.method === "OPTIONS") return options(request);
  if (request.method !== "POST") return json(request, { error: "method_not_allowed" }, 405);

  const body = await jsonBody(request);
  const deviceId = typeof body?.device_id === "string" ? body.device_id.trim() : "";
  const deviceSecret = typeof body?.device_secret === "string" ? body.device_secret : "";
  const partnerId = typeof body?.partner_id === "string" ? body.partner_id.trim() : "";
  const requestedRoom = typeof body?.room_id === "string" ? body.room_id : "";
  const expectedRoom = canonicalRoomId(deviceId, partnerId);

  // Do not accept an arbitrary room name: it must be derived from the two
  // devices the Harbor registry says are paired.
  if (!deviceId || !deviceSecret || !partnerId || !expectedRoom || requestedRoom !== expectedRoom) {
    return json(request, { error: "invalid_media_request" }, 400);
  }

  const auth = await authorizeWithHarbor(deviceId, deviceSecret, requestedRoom, partnerId);
  if (
    !auth ||
    auth.authorized !== true ||
    auth.role !== "peer" ||
    auth.partner_id !== partnerId
  ) {
    return json(request, { error: "media_not_authorized" }, 403);
  }

  const jwtSecret = Deno.env.get("MEDIA_JWT_SECRET");
  if (!jwtSecret) return json(request, { error: "media_not_configured" }, 503);

  const now = Math.floor(Date.now() / 1000);
  const exp = now + configuredTtl();
  const token = await signMediaJwt(
    {
      sub: deviceId,
      role: "authenticated",
      aud: "authenticated",
      iss: "harbor-media",
      media_topic: topicForRoom(requestedRoom),
      media_peer_id: partnerId,
      iat: now,
      exp,
    },
    jwtSecret,
  );

  // The device secret and registry response never leave this function.
  return json(request, {
    access_token: token,
    token_type: "Bearer",
    expires_at: exp,
    room_id: requestedRoom,
    media_topic: topicForRoom(requestedRoom),
  });
}

Deno.serve(handle);
