/**
 * Harbor Worker entry — a thin request router that delegates to the two Durable Objects.
 *
 *   HTTP routes  → HarborRegistry RPC (with HarborPair RPC side-effects for /partner & /unpair)
 *   GET /ws      → HarborRegistry.verifyDevice, then forward the upgrade to HarborPair
 *
 * The worker holds no secrets and no per-connection state; it only authenticates
 * (via the Registry) and routes (to the per-pair DO). CORS is permissive to match the
 * FastAPI relay (which allowed all origins) so the Tauri client keeps working.
 */
import { verifySecret } from "./util";
import {
  MeInfo,
  PartnerInfo,
  PairResponse,
  MediaAuthorizationRequest,
  MediaAuthorizationResponse,
  RegisterResponse,
  RegisterRequest,
  PairRequest,
  UnpairRequest,
  UnpairResponse,
  UpdateProfileRequest,
  MobileCodeRequest,
  MobileCodeResponse,
  ConnectMobileRequest,
  ConnectMobileResponse,
} from "./protocol";

// Re-export the Durable Object classes so the Worker bundle (main = src/index.ts)
// can resolve `HarborRegistry` / `HarborPair` named in wrangler.jsonc `exports` +
// `durable_objects.bindings`. Without this, `wrangler types` renders the bindings as
// bare `DurableObjectNamespace` (RPC methods untyped) and the runtime can't
// instantiate the DOs.
export { HarborRegistry } from "./registry";
export { HarborPair } from "./pair";

/* ─────────────────────────── helpers ────────────────────────────── */

const JSON_HEADERS: Record<string, string> = { "Content-Type": "application/json" };

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), { status, headers: JSON_HEADERS });
}

/** Permissive CORS (the FastAPI relay allowed all origins; preserve for the Tauri client).
 *  Harbor's client is a Tauri desktop app whose WebView origin varies, so allow all. */
function cors(): Record<string, string> {
  return {
    "Access-Control-Allow-Origin": "*",
    "Access-Control-Allow-Methods": "GET, POST, OPTIONS",
    "Access-Control-Allow-Headers": "Authorization, Content-Type",
    "Access-Control-Max-Age": "86400",
  };
}

/** Read + validate a JSON body against a predicate; returns the parsed value or a 400. */
async function readJson<T>(
  request: Request,
  validate: (v: unknown) => T | null,
): Promise<{ ok: true; value: T } | { ok: false; response: Response }> {
  let parsed: unknown;
  try {
    parsed = await request.json();
  } catch {
    return { ok: false, response: json({ detail: "invalid_json" }, 400) };
  }
  const value = validate(parsed);
  if (value === null) return { ok: false, response: json({ detail: "invalid_body" }, 400) };
  return { ok: true, value };
}

/** Map a DO-thrown PairError to its matching HTTP response.
 *
 *  Errors thrown inside a Durable Object RPC call are re-created in the Worker's
 *  isolate as a generic `Error` — the custom `PairError` class does NOT survive
 *  the RPC round-trip (so `e instanceof PairError` is always false here). What
 *  /does/ survive is the enumerable `.status` (a number) and `.message`, set as
 *  plain properties on the thrown object. Read them structurally instead of via
 *  `instanceof`, and fall back to 500 only when no usable status is present. */
function handleError(e: unknown): Response {
  const status = (e as { status?: unknown })?.status;
  if (typeof status === "number" && status >= 400 && status < 600) {
    return json({ detail: e instanceof Error ? e.message : String(e) }, status);
  }
  // Don't leak internals; 500 for anything unexpected.
  console.error("harbor worker error", e instanceof Error ? e.message : String(e));
  return json({ detail: "internal_error" }, 500);
}

const registry = (env: Env) => env.HARBOR_REGISTRY.get(env.HARBOR_REGISTRY.idFromName("harbor-registry"));

/* ────────────────────────── validators ───────────────────────────── */

const isStr = (v: unknown, min = 1): v is string => typeof v === "string" && v.length >= min;
const isOptStr = (v: unknown): v is string | null =>
  v === undefined || v === null || typeof v === "string";

function validateRegister(v: unknown): RegisterRequest | null {
  if (!v || typeof v !== "object") return null;
  const o = v as Record<string, unknown>;
  if (!isStr(o["device_id"], 8)) return null;
  if (!isOptStr(o["public_key"]) || !isOptStr(o["avatar"])) return null;
  return { device_id: o["device_id"], public_key: o["public_key"] ?? null, avatar: o["avatar"] ?? null };
}

function validatePair(v: unknown): PairRequest | null {
  if (!v || typeof v !== "object") return null;
  const o = v as Record<string, unknown>;
  if (!isStr(o["device_id"]) || !isStr(o["device_secret"]) || !isStr(o["partner_code"])) return null;
  return { device_id: o["device_id"], device_secret: o["device_secret"], partner_code: o["partner_code"] };
}

function validateMediaAuthorization(v: unknown): MediaAuthorizationRequest | null {
  if (!v || typeof v !== "object") return null;
  const o = v as Record<string, unknown>;
  if (
    !isStr(o["device_id"]) ||
    !isStr(o["device_secret"]) ||
    !isStr(o["partner_id"]) ||
    !isStr(o["room_id"])
  ) {
    return null;
  }
  return {
    device_id: o["device_id"],
    device_secret: o["device_secret"],
    partner_id: o["partner_id"],
    room_id: o["room_id"],
  };
}

function canonicalMediaRoom(deviceId: string, partnerId: string): string {
  const [first, second] = [deviceId.trim(), partnerId.trim()].sort();
  return `pair:${encodeURIComponent(first)}:${encodeURIComponent(second)}`;
}

function validateInternalBearer(request: Request, expected: string | undefined): boolean {
  const value = request.headers.get("Authorization") ?? "";
  const match = /^Bearer\s+([^\s]+)$/i.exec(value);
  return Boolean(expected && match && verifySecret(expected, match[1]));
}

function validateProfile(v: unknown): UpdateProfileRequest | null {
  if (!v || typeof v !== "object") return null;
  const o = v as Record<string, unknown>;
  if (!isStr(o["device_id"]) || !isStr(o["device_secret"])) return null;
  // display_name may be a string or null (omitted is fine → null, set server-side)
  const displayName = o["display_name"];
  if (displayName !== undefined && displayName !== null && typeof displayName !== "string") return null;
  if (!isOptStr(o["public_key"]) || !isOptStr(o["avatar"])) return null;
  return {
    device_id: o["device_id"],
    device_secret: o["device_secret"],
    display_name: (displayName ?? null) as string | null,
    public_key: o["public_key"] ?? null,
    avatar: o["avatar"] ?? null,
  };
}

function validateUnpair(v: unknown): UnpairRequest | null {
  if (!v || typeof v !== "object") return null;
  const o = v as Record<string, unknown>;
  if (!isStr(o["device_id"]) || !isStr(o["device_secret"])) return null;
  return { device_id: o["device_id"], device_secret: o["device_secret"] };
}

function validateMobileCode(v: unknown): MobileCodeRequest | null {
  if (!v || typeof v !== "object") return null;
  const o = v as Record<string, unknown>;
  if (!isStr(o["device_id"]) || !isStr(o["device_secret"])) return null;
  return { device_id: o["device_id"], device_secret: o["device_secret"] };
}

function validateConnectMobile(v: unknown): ConnectMobileRequest | null {
  if (!v || typeof v !== "object") return null;
  const o = v as Record<string, unknown>;
  if (!isStr(o["device_id"]) || !isStr(o["device_secret"]) || !isStr(o["mobile_code"])) return null;
  return {
    device_id: o["device_id"],
    device_secret: o["device_secret"],
    mobile_code: o["mobile_code"],
  };
}

/* ─────────────────────────── router ──────────────────────────────── */

export default {
  async fetch(request: Request, env: Env, ctx: ExecutionContext): Promise<Response> {
    const url = new URL(request.url);

    // CORS preflight for any route.
    if (request.method === "OPTIONS") {
      return new Response(null, { status: 204, headers: cors() });
    }

    // Attach CORS to real responses too, so the Tauri WebView isn't blocked.
    const withCors = (res: Response): Response => {
      const h = new Headers(res.headers);
      for (const [k, v] of Object.entries(cors())) h.set(k, v);
      return new Response(res.body, { status: res.status, headers: h });
    };

    try {
      // /health — direct, no DO.
      if (request.method === "GET" && url.pathname === "/health") {
        return withCors(json({ status: "ok" }));
      }

      // /ws — authenticate via the Registry, then forward the upgrade to HarborPair.
      if (request.method === "GET" && url.pathname === "/ws") {
        const deviceId = url.searchParams.get("device_id");
        const secret = url.searchParams.get("secret");
        if (!deviceId || !secret) return withCors(json({ detail: "unauthorized" }, 401));
        const reg = registry(env);
        const v = await reg.verifyDevice(deviceId, secret);
        if (!v.ok || !v.pair_key) return withCors(json({ detail: "unauthorized" }, 401));
        const pair = env.HARBOR_PAIR.get(env.HARBOR_PAIR.idFromName(v.pair_key));
        // Forward the raw upgraded request; HarborPair.fetch upgrades + returns 101.
        const doRes = await pair.fetch(request);
        return doRes; // 101 responses don't take CORS headers.
      }

      // Private server-to-server authorization used by the Supabase media-session
      // function. The bearer is a Cloudflare secret, never a device credential.
      if (request.method === "POST" && url.pathname === "/media-authorize") {
        const internalToken = env.HARBOR_MEDIA_AUTH_TOKEN;
        if (!internalToken) return withCors(json({ detail: "media_auth_not_configured" }, 503));
        if (!validateInternalBearer(request, internalToken)) {
          return withCors(json({ detail: "unauthorized" }, 401));
        }
        const body = await readJson(request, validateMediaAuthorization);
        if (!body.ok) return withCors(body.response);
        if (body.value.room_id !== canonicalMediaRoom(body.value.device_id, body.value.partner_id)) {
          return withCors(json({ detail: "invalid_media_room" }, 400));
        }
        const result: MediaAuthorizationResponse = await registry(env).authorizeMedia(
          body.value.device_id,
          body.value.device_secret,
          body.value.partner_id,
        );
        return withCors(json(result));
      }

      // HTTP routes → Registry RPC.
      if (request.method === "POST" && url.pathname === "/register") {
        const body = await readJson(request, validateRegister);
        if (!body.ok) return withCors(body.response);
        const r: RegisterResponse = await registry(env).register(
          body.value.device_id,
          body.value.public_key,
          body.value.avatar,
        );
        return withCors(json(r));
      }

      if (request.method === "POST" && url.pathname === "/pair") {
        const body = await readJson(request, validatePair);
        if (!body.ok) return withCors(body.response);
        const r: PairResponse = await registry(env).pair(
          body.value.device_id,
          body.value.device_secret,
          body.value.partner_code,
        );
        return withCors(json(r));
      }

      if (request.method === "POST" && url.pathname === "/profile") {
        const body = await readJson(request, validateProfile);
        if (!body.ok) return withCors(body.response);
        await registry(env).setProfile(
          body.value.device_id,
          body.value.device_secret,
          body.value.display_name,
          body.value.public_key,
          body.value.avatar,
        );
        return withCors(json({ ok: true }));
      }

      if (request.method === "POST" && url.pathname === "/mobile_code") {
        const body = await readJson(request, validateMobileCode);
        if (!body.ok) return withCors(body.response);
        const r: MobileCodeResponse = await registry(env).mintMobileCode(
          body.value.device_id,
          body.value.device_secret,
        );
        return withCors(json(r));
      }

      if (request.method === "POST" && url.pathname === "/connect_mobile") {
        const body = await readJson(request, validateConnectMobile);
        if (!body.ok) return withCors(body.response);
        const r: ConnectMobileResponse = await registry(env).bindObserver(
          body.value.device_id,
          body.value.device_secret,
          body.value.mobile_code,
        );
        return withCors(json(r));
      }

      if (request.method === "GET" && url.pathname === "/partner") {
        const deviceId = url.searchParams.get("device_id");
        const secret = url.searchParams.get("secret");
        if (!deviceId || !secret) return withCors(json({ detail: "unauthorized" }, 401));
        const r: PartnerInfo = await registry(env).getPartnerInfo(deviceId, secret);
        return withCors(json(r));
      }

      if (request.method === "POST" && url.pathname === "/unpair") {
        const body = await readJson(request, validateUnpair);
        if (!body.ok) return withCors(body.response);
        const r: UnpairResponse = await registry(env).unpair(
          body.value.device_id,
          body.value.device_secret,
        );
        // Drop the internal-only field before returning to the client.
        const { ok, pairing_code } = r;
        return withCors(json({ ok, pairing_code }));
      }

      if (request.method === "GET" && url.pathname === "/me") {
        const deviceId = url.searchParams.get("device_id");
        const secret = url.searchParams.get("secret");
        if (!deviceId || !secret) return withCors(json({ detail: "unauthorized" }, 401));
        const r: MeInfo = await registry(env).getMe(deviceId, secret);
        return withCors(json(r));
      }

      return withCors(new Response("Not Found", { status: 404 }));
    } catch (e) {
      return withCors(handleError(e));
    }
  },
} satisfies ExportedHandler<Env>;
