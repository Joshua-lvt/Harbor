/**
 * Harbor wire protocol — shared types for the Cloudflare Worker + Durable Objects.
 *
 * This is the single source of truth for the shapes that travel between the Harbor
 * client and the Worker. The envelope names and fields are lifted verbatim from the
 * FastAPI relay (`server/app/ws.py` + `server/app/models.py`) so an existing client
 * keeps working without a protocol change — only the backend URL moves.
 *
 * Two invariants the relay has always enforced and that the Worker preserves:
 *
 *  - `chat` is a discriminated shape: under E2E the envelope carries an opaque base64
 *    `enc` string (libsodium crypto_box_seal) with NO `text`/`image` keys — the server
 *    never decrypts and forwards it verbatim. Without `enc`, the legacy plaintext path
 *    carries `text` + an optional `image` data URL. A parsed message saturates the
 *    union below only when it has `id` plus exactly one of `enc` or `text`.
 *
 *  - the server only ever routes metadata + public keys + opaque ciphertext. Private
 *    keys and plaintext (for the E2E path) never touch the server.
 */

/* ───────────────────────── WebSocket messages ───────────────────────── */

/** Events the client sends to the Worker over the WebSocket. */
export type ClientMessage =
  | { type: "heartbeat" }
  | { type: "presence"; state: "online" | "away" }
  | { type: "typing"; state?: "start" | "stop" }
  | { type: "activity"; app: string | null }
  | { type: "voice_signal"; kind: "offer" | "answer" | "ice"; data: unknown }
  | ({ type: "chat"; id: string } & (
      | { enc: string }
      | { text: string; image?: string }
    ))
  | { type: "last_seen" }
  // Additive real-time push types (forwarded verbatim, exactly like `activity`).
  // The live Worker (pre-deploy) drops these as `unknown_message_type` → the
  // clients fall back to cold-start HTTP sync for the affected field; no crash.
  | { type: "profile_update"; display_name: string | null; avatar: string | null }
  | { type: "activity_icon"; app: string; icon: string | null };

/** Validated voice-signal kinds (WebRTC signaling only — never media). */
export type VoiceSignalKind = "offer" | "answer" | "ice";

/** Events the Worker sends to the client over the WebSocket. */
export type ServerMessage =
  | { type: "ack"; id: string; delivered: boolean }
  | {
      type: "presence";
      device_id: string;
      state: "online" | "away" | "offline";
      ts: number;
      last_seen?: number;
    }
  | { type: "typing"; device_id: string; state: "start" | "stop"; ts: number }
  | { type: "activity"; device_id: string; app: string | null; ts: number }
  | {
      type: "voice_signal";
      device_id: string;
      kind: VoiceSignalKind;
      data: unknown;
      ts: number;
    }
  | ({ type: "chat"; id: string; from: string; ts: number } & (
      | { enc: string }
      | { text: string; image?: string }
    ))
  | {
      type: "last_seen";
      device_id: string;
      last_seen: number | null;
      presence: string;
    }
  | { type: "unpaired"; pairing_code: string; ts: number }
  | { type: "error"; reason: string }
  // Additive real-time push types relayed verbatim with device_id + ts added.
  | { type: "profile_update"; device_id: string; display_name: string | null; avatar: string | null; ts: number }
  | { type: "activity_icon"; device_id: string; app: string; icon: string | null; ts: number };

/* ───────────────────────────── HTTP bodies ──────────────────────────── */

/** POST /register — client-generated device_id + optional pubkey/avatar. */
export interface RegisterRequest {
  device_id: string;
  public_key?: string | null;
  avatar?: string | null;
}

export interface RegisterResponse {
  pairing_code: string;
  device_secret: string;
}

/** POST /pair — present the partner's code to link the two devices. */
export interface PairRequest {
  device_id: string;
  device_secret: string;
  partner_code: string;
}

export interface PairResponse {
  partner_device_id: string;
  partner_name: string | null;
  partner_public_key: string | null;
  partner_avatar: string | null;
}

/**
 * POST /profile — update display_name and/or public_key and/or avatar.
 *
 * Avatar semantics (faithful to FastAPI):
 *  - `undefined` / omitted → don't touch the stored value (distinct from clearing)
 *  - `""`           → clear the stored avatar
 *  - any data URL   → set the stored avatar
 * `public_key`/`display_name` follow the trivial "set when provided" rule.
 */
export interface UpdateProfileRequest {
  device_id: string;
  device_secret: string;
  display_name?: string | null;
  public_key?: string | null;
  avatar?: string | null;
}

/** POST /unpair — break the current pairing bilaterally. */
export interface UnpairRequest {
  device_id: string;
  device_secret: string;
}

export interface UnpairResponse {
  ok: true;
  /** The caller's freshly reissued pairing code (so they can pair with someone new). */
  pairing_code: string;
}

/**
 * GET /partner — the passive partner's cold-start lookup of the caller's static info.
 * `presence` + `last_seen` are live data owned by HarborPair and are merged in by the
 * Worker after the Registry returns this static portion.
 */
export interface PartnerInfo {
  partner_device_id: string;
  partner_name: string | null;
  presence: string;
  last_seen: number | null;
  partner_public_key: string | null;
  partner_avatar: string | null;
}

/** GET /me — the caller's own state, for cold-start recovery. */
export interface MeInfo {
  pairing_code: string | null;
  partner_id: string | null;
  display_name: string | null;
}

/* ───────────────────────── Validation helpers ──────────────────────── */

/**
 * Narrow an unknown parsed JSON value to a typed ClientMessage, or return an
 * `{type:"error"}` ServerMessage describing why it was rejected. The Worker/DO feed it
 * the raw `JSON.parse` output; bad shapes come back as an `error` so the caller can send
 * that back to the client without dropping the socket (faithful to FastAPI's
 * swallow-on-error per-message behavior).
 *
 * Intentionally permissive where FastAPI was: `typing.state` defaults to "start"
 * (`ws.py:144`), `chat.id` defaults to a server timestamp when missing (`ws.py:176`),
 * unknown presence states are silently ignored (the caller decides), and `image` is only
 * attached on the plaintext path when it's a `data:image/` URL (`ws.py:209`).
 */
export type ValidatedClient =
  | { ok: true; msg: ClientMessage }
  | { ok: false; msg: Extract<ServerMessage, { type: "error" }> };

/** Validate a single inbound WS message. See `ValidatedClient` for the contract. */
export function validateClientMessage(raw: unknown): ValidatedClient {
  const err = (reason: string): ValidatedClient => ({ ok: false, msg: { type: "error", reason } });

  if (raw === null || typeof raw !== "object") return err("malformed_message");
  const m = raw as Record<string, unknown>;
  const t = m["type"];

  switch (t) {
    case "heartbeat":
      return { ok: true, msg: { type: "heartbeat" } };

    case "presence": {
      const state = m["state"];
      if (state !== "online" && state !== "away") return err("bad_presence_state");
      return { ok: true, msg: { type: "presence", state } };
    }

    case "typing": {
      const s = m["state"];
      const state: "start" | "stop" = s === "stop" ? "stop" : "start"; // default "start"
      return { ok: true, msg: { type: "typing", state } };
    }

    case "activity": {
      const app = m["app"];
      // FastAPI forwarded whatever `app` was (including null). Strings or null only.
      if (typeof app !== "string" && app !== null) return err("bad_activity");
      return { ok: true, msg: { type: "activity", app } };
    }

    case "voice_signal": {
      const kind = m["kind"];
      const data = m["data"];
      if (kind !== "offer" && kind !== "answer" && kind !== "ice") return err("bad_signal_kind");
      // Signaling payloads are opaque to the relay: offer/answer are SDP objects,
      // ICE candidates are candidate objects — the relay forwards them verbatim
      // (audio is P2P, the relay only relays signaling). Reject only a missing
      // `data`; any JSON value (string OR object) is forwarded untouched, matching
      // the FastAPI relay's `msg.get("data")` passthrough (ws.py:171). Tightening
      // this to `typeof data === "string"` here silently dropped every real
      // WebRTC object the client sends → offer never reached the partner →
      // infinite reconnect loop (voice.ts re-offers every 8s forever).
      if (data === undefined) return err("bad_signal_data");
      return { ok: true, msg: { type: "voice_signal", kind, data } };
    }

    case "chat": {
      const id = typeof m["id"] === "string" ? m["id"] : null;
      if (id === null) return err("missing_chat_id");
      const enc = m["enc"];
      if (typeof enc === "string" && enc.length > 0) {
        return { ok: true, msg: { type: "chat", id, enc } };
      }
      const text = m["text"];
      if (typeof text === "string") {
        const image = m["image"];
        if (image !== undefined && typeof image !== "string") return err("bad_image");
        // Only data: URLs are forwarded on the plaintext path (ws.py:209).
        const img =
          typeof image === "string" && image.startsWith("data:image/") ? image : undefined;
        return { ok: true, msg: { type: "chat", id, text, ...(img ? { image: img } : {}) } };
      }
      // Neither enc nor a usable text → treat as plaintext with empty text (ws.py:200).
      return { ok: true, msg: { type: "chat", id, text: "" } };
    }

    case "last_seen":
      return { ok: true, msg: { type: "last_seen" } };

    case "profile_update": {
      const display_name = m["display_name"];
      if (display_name !== null && typeof display_name !== "string")
        return err("bad_profile_name");
      const avatar = m["avatar"];
      if (avatar !== null && typeof avatar !== "string")
        return err("bad_profile_avatar");
      if (typeof avatar === "string" && avatar.length > 0 && !avatar.startsWith("data:image/"))
        return err("bad_profile_avatar");
      return {
        ok: true,
        msg: { type: "profile_update", display_name, avatar },
      };
    }

    case "activity_icon": {
      const app = m["app"];
      if (typeof app !== "string" || app.length === 0) return err("bad_icon_app");
      const icon = m["icon"];
      if (icon !== null && typeof icon !== "string") return err("bad_icon_data");
      if (typeof icon === "string" && icon.length > 0 && !icon.startsWith("data:image/"))
        return err("bad_icon_data");
      return { ok: true, msg: { type: "activity_icon", app, icon } };
    }

    default:
      return err("unknown_message_type");
  }
}

/* ───────────────────────── Pair-error plumbing ─────────────────────── */

/** A business error carrying an HTTP-ish status code (faithful to pairing.PairError). */
export class PairError extends Error {
  status: number;
  constructor(detail: string, status = 400) {
    super(detail);
    this.status = status;
    this.name = "PairError";
  }
}
