import { bearerToken, json, jsonBody, options } from "../_shared/http.ts";
import { verifyMediaJwt } from "../_shared/jwt.ts";

interface TurnServer {
  urls: string | string[];
  username: string;
  credential: string;
}

const MAX_TURN_TTL_SECONDS = 60 * 60;

function configuredServers(): TurnServer[] | null {
  const raw = Deno.env.get("TURN_ICE_SERVERS_JSON");
  if (!raw) return null;
  try {
    const value: unknown = JSON.parse(raw);
    if (!Array.isArray(value) || value.length === 0) return null;
    const servers: TurnServer[] = [];
    for (const item of value) {
      if (!item || typeof item !== "object" || Array.isArray(item)) return null;
      const server = item as Record<string, unknown>;
      const urls = server.urls;
      const validUrls =
        typeof urls === "string" && urls.startsWith("turn:")
          ? urls
          : Array.isArray(urls) &&
              urls.length > 0 &&
              urls.every((url) => typeof url === "string" && url.startsWith("turn:"))
            ? urls
            : null;
      if (
        !validUrls ||
        typeof server.username !== "string" ||
        !server.username ||
        typeof server.credential !== "string" ||
        !server.credential
      ) {
        return null;
      }
      servers.push({
        urls: validUrls,
        username: server.username,
        credential: server.credential,
      });
    }
    return servers;
  } catch {
    return null;
  }
}

function configuredTtl(): number {
  const raw = Number(Deno.env.get("TURN_TTL_SECONDS") ?? "300");
  if (!Number.isFinite(raw) || raw < 60) return 300;
  return Math.min(Math.floor(raw), MAX_TURN_TTL_SECONDS);
}

async function handle(request: Request): Promise<Response> {
  if (request.method === "OPTIONS") return options(request);
  if (request.method !== "POST") return json(request, { error: "method_not_allowed" }, 405);

  const token = bearerToken(request);
  const jwtSecret = Deno.env.get("MEDIA_JWT_SECRET");
  const body = await jsonBody(request);
  const roomId = typeof body?.room_id === "string" ? body.room_id : "";
  const deviceId = typeof body?.device_id === "string" ? body.device_id : "";
  const partnerId = typeof body?.partner_id === "string" ? body.partner_id : "";
  if (!token || !jwtSecret || !roomId || !deviceId || !partnerId) {
    return json(request, { error: "media_not_authorized" }, 403);
  }

  const claims = await verifyMediaJwt(token, jwtSecret);
  const expectedTopic = `harbor:voice:${roomId}`;
  if (
    !claims ||
    claims.sub !== deviceId ||
    claims.media_peer_id !== partnerId ||
    claims.media_topic !== expectedTopic
  ) {
    return json(request, { error: "media_not_authorized" }, 403);
  }

  const servers = configuredServers();
  if (!servers) return json(request, { error: "turn_not_configured" }, 503);

  const now = Math.floor(Date.now() / 1000);
  const expiresAt = now + configuredTtl();
  return json(request, {
    // These are short-lived relay credentials, not the provider's API secret.
    ice_servers: servers,
    expires_at: expiresAt,
  });
}

Deno.serve(handle);
