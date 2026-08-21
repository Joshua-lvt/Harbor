/**
 * ICE configuration provider.
 *
 * STUN is public and safe to keep as a default. TURN credentials are always
 * fetched from an authenticated Edge Function and cached only in memory; this
 * module deliberately contains no TURN username or password.
 */
import { SUPABASE_ANON_KEY, SUPABASE_URL } from "../lib/supabase";

export interface IceServer {
  urls: string | string[];
  username?: string;
  credential?: string;
}

export interface IceConfiguration {
  iceServers: IceServer[];
}

export interface IceConfigurationResult {
  configuration: IceConfiguration;
  usedTurn: boolean;
  expiresAtMs: number | null;
  warning: string | null;
}

export interface TurnRequest {
  accessToken: string;
  roomId: string;
  deviceId: string;
  partnerId: string;
}

interface TurnResponse {
  ice_servers?: unknown;
  expires_at?: unknown;
  ttl_seconds?: unknown;
}

interface CachedConfiguration {
  result: IceConfigurationResult;
  cacheKey: string;
}

/** Public discovery only; no relay credentials are embedded here. */
export const DEFAULT_ICE_SERVERS: readonly IceServer[] = [
  { urls: "stun:stun.l.google.com:19302" },
  { urls: "stun:stun.cloudflare.com:3478" },
];

const REFRESH_SKEW_MS = 30_000;
const MAX_TTL_MS = 24 * 60 * 60 * 1000;

export interface IceProviderOptions {
  endpoint?: string;
  fetchImpl?: typeof fetch;
  now?: () => number;
  baseServers?: readonly IceServer[];
}

export class IceConfigurationProvider {
  private readonly endpoint: string;
  private readonly fetchImpl: typeof fetch;
  private readonly now: () => number;
  private readonly baseServers: readonly IceServer[];
  private cached: CachedConfiguration | null = null;

  constructor(options: IceProviderOptions = {}) {
    const endpoint = options.endpoint ?? `${SUPABASE_URL}/functions/v1/turn-credentials`;
    if (new URL(endpoint).protocol !== "https:") {
      throw new Error("TURN endpoint must use HTTPS");
    }
    this.endpoint = endpoint;
    this.fetchImpl = options.fetchImpl ?? fetch;
    this.now = options.now ?? (() => Date.now());
    this.baseServers = options.baseServers ?? DEFAULT_ICE_SERVERS;
  }

  /**
   * Return TURN-backed ICE when authorized and available. A transient TURN
   * failure returns a STUN-only configuration so direct/LAN calls still work;
   * `warning` remains observable by the caller instead of silently hiding the
   * degraded transport.
   */
  async getConfiguration(request: TurnRequest | null): Promise<IceConfigurationResult> {
    const fallback = (): IceConfigurationResult => ({
      configuration: { iceServers: this.cloneServers(this.baseServers) },
      usedTurn: false,
      expiresAtMs: null,
      warning: request ? "TURN indisponível; tentando conexão direta" : "TURN requer autenticação",
    });

    if (!request?.accessToken || !request.roomId || !request.deviceId || !request.partnerId) {
      return fallback();
    }

    const cacheKey = `${request.roomId}:${request.deviceId}:${request.partnerId}`;
    if (this.cached?.cacheKey === cacheKey && this.isFresh(this.cached.result.expiresAtMs)) {
      return this.cached.result;
    }

    try {
      const response = await this.fetchImpl(this.endpoint, {
        method: "POST",
        headers: {
          apikey: SUPABASE_ANON_KEY,
          Authorization: `Bearer ${request.accessToken}`,
          "Content-Type": "application/json",
        },
        body: JSON.stringify({
          room_id: request.roomId,
          device_id: request.deviceId,
          partner_id: request.partnerId,
        }),
      });
      if (!response.ok) throw new Error(`TURN request failed (${response.status})`);
      const payload = (await response.json()) as TurnResponse;
      const servers = normalizeIceServers(payload.ice_servers);
      const expiresAtMs = parseExpiry(payload, this.now());
      if (!servers.length || !expiresAtMs || !this.isFresh(expiresAtMs)) {
        throw new Error("TURN response is invalid or expired");
      }
      const result: IceConfigurationResult = {
        configuration: { iceServers: [...this.cloneServers(this.baseServers), ...servers] },
        usedTurn: true,
        expiresAtMs,
        warning: null,
      };
      this.cached = { cacheKey, result };
      return result;
    } catch (error) {
      const result = fallback();
      return {
        ...result,
        warning: `${result.warning}: ${error instanceof Error ? error.message : "erro desconhecido"}`,
      };
    }
  }

  clear(): void {
    this.cached = null;
  }

  private isFresh(expiresAtMs: number | null): boolean {
    return expiresAtMs != null && expiresAtMs - this.now() > REFRESH_SKEW_MS;
  }

  private cloneServers(servers: readonly IceServer[]): IceServer[] {
    return servers.map((server) => ({
      urls: Array.isArray(server.urls) ? [...server.urls] : server.urls,
      ...(server.username ? { username: server.username } : {}),
      ...(server.credential ? { credential: server.credential } : {}),
    }));
  }
}

function normalizeIceServers(value: unknown): IceServer[] {
  if (!Array.isArray(value)) return [];
  const result: IceServer[] = [];
  for (const item of value) {
    if (!item || typeof item !== "object") continue;
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
    if (!validUrls || typeof server.username !== "string" || !server.username) continue;
    if (typeof server.credential !== "string" || !server.credential) continue;
    result.push({ urls: validUrls, username: server.username, credential: server.credential });
  }
  return result;
}

function parseExpiry(payload: TurnResponse, nowMs: number): number | null {
  if (typeof payload.expires_at === "number" && Number.isFinite(payload.expires_at)) {
    // The Edge Function contract uses epoch seconds; accept epoch ms only when
    // the magnitude makes that unambiguous for defensive compatibility.
    const expiresAtMs = payload.expires_at < 10_000_000_000 ? payload.expires_at * 1000 : payload.expires_at;
    if (expiresAtMs > nowMs && expiresAtMs - nowMs <= MAX_TTL_MS) return expiresAtMs;
  }
  if (typeof payload.ttl_seconds === "number" && Number.isFinite(payload.ttl_seconds)) {
    const ttlMs = payload.ttl_seconds * 1000;
    if (ttlMs > REFRESH_SKEW_MS && ttlMs <= MAX_TTL_MS) return nowMs + ttlMs;
  }
  return null;
}
