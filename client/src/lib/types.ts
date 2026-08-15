/**
 * Domain types shared across the client. Envelopes mirror the relay's WS schema
 * (see server/app/ws.py) so the frontend validates what the server sends.
 */
export type PresenceState = "online" | "away" | "offline";

/** Events received over the WebSocket from the relay. The `chat` variant is a
 *  discriminated union: under E2E the envelope carries an opaque base64 `enc`
 *  string (libsodium crypto_box_seal) with NO `text`/`image` keys — narrow on
 *  `enc` for the encrypted path, on `text` for the legacy plaintext fallback. */
export type ServerEvent =
  | { type: "presence"; device_id: string; state: PresenceState; ts: number; last_seen?: number }
  | { type: "chat"; id: string; from: string; text: string; ts: number; image?: string }
  | { type: "chat"; id: string; from: string; enc: string; ts: number }
  | { type: "ack"; id: string; delivered: boolean }
  | { type: "typing"; device_id: string; state: "start" | "stop"; ts: number }
  | { type: "last_seen"; device_id: string; last_seen: number | null; presence: PresenceState }
  | { type: "activity"; device_id: string; app: string | null; ts: number }
  | { type: "voice_signal"; device_id: string; kind: "offer" | "answer" | "ice"; data: unknown; ts: number }
  /** Additive real-time push — partner published a new name+avatar (the HTTP
   *  POST /profile stays the persistent source of truth; this is the live opt). */
  | { type: "profile_update"; device_id: string; display_name: string | null; avatar: string | null; ts: number }
  /** Additive one-per-exe icon push — the exe NAME keeps traveling in `activity`;
   *  the icon payload goes once. null = generated fallback confirmed. */
  | { type: "activity_icon"; device_id: string; app: string; icon: string | null; ts: number }
  /** The partner unpaired us — the pairing is gone, and `pairing_code` is OUR
   *  newly reissued code (so we can pair with someone new). Return to pairing. */
  | { type: "unpaired"; pairing_code: string; ts: number };

/** Events the client sends to the relay. `chat` splits the same way: the E2E
 *  send carries `enc` and omits `text`; the plaintext fallback carries `text`. */
export type ClientEvent =
  | { type: "chat"; id: string; text: string; image?: string }
  | { type: "chat"; id: string; enc: string }
  | { type: "presence"; state: "online" | "away" }
  | { type: "typing"; state: "start" | "stop" }
  | { type: "heartbeat" }
  | { type: "last_seen" }
  | { type: "activity"; app: string | null }
  | {
      type: "voice_signal";
      kind: "offer" | "answer" | "ice";
      data: unknown;
    }
  /** Outbound real-time profile push (mirrors the inbound `profile_update`
   *  ServerEvent minus the relay-added device_id/ts). */
  | { type: "profile_update"; display_name: string | null; avatar: string | null }
  /** Outbound one-per-exe icon push (mirrors the inbound ServerEvent). */
  | { type: "activity_icon"; app: string; icon: string | null };

/** Local persisted message. `from_me` orientates the bubble. `image`, when
 *  present, is a compressed JPEG data URL (sent inline in the chat envelope).
 *  Always stored as PLAINTEXT locally — E2E encryption is transport-only here;
 *  at-rest encryption is a deferred milestone. */
export interface StoredMessage {
  id: string;
  partner_id: string;
  text: string;
  from_me: boolean;
  created_at: number;
  status: "sending" | "delivered";
  image?: string | null;
}

/** Persisted device identity + relay config (Tauri store, identity.json).
 *  The `device_*` keys are this device's X25519 keypair (private never leaves
 *  the client); `partner_pubkey` is the partner's public key, learned from the
 *  /pair response (caller) or GET /partner (passive partner), refreshed on cold
 *  start. `my_avatar`/`partner_avatar` are base64 JPEG data URLs (profile
 *  photos) following the same publish/propagate contract. All optional — old
 *  stores that predate these load with them absent. */
export interface Identity {
  device_id: string;
  device_secret: string;
  pairing_code: string | null;
  relay_url: string;
  partner_id?: string | null;
  partner_name?: string | null;
  my_name?: string | null;
  /** This device's X25519 public key (base64). Published to the relay. */
  device_pubkey?: string | null;
  /** This device's X25519 private key (base64). NEVER sent to the relay. */
  device_privkey?: string | null;
  /** The partner's X25519 public key (base64). Received at pair/cold-start. */
  partner_pubkey?: string | null;
  /** This device's profile photo (base64 JPEG data URL). Source of truth is the
   *  local store (like device_privkey); published to the relay via /profile so
   *  the partner can render it. null/absent → shark mascot fallback. */
  my_avatar?: string | null;
  /** The partner's profile photo (base64 JPEG data URL). Received at pair /
   *  cold-start (like partner_pubkey), refreshed on cold start. */
  partner_avatar?: string | null;
}

export interface Settings {
  notify_on_online: boolean;
  notify_on_away: boolean;
  notify_on_offline: boolean;
  notify_on_message: boolean;
  away_after_minutes: number;
  relay_url: string;
  autostart_enabled: boolean;
  /** Share my current foreground app with the partner (Discord-style "using …").
   *  Off = activity mirroring is fully disabled: nothing is broadcast and the
   *  partner sees no app for me. See useActivity.ts. */
  share_activity: boolean;
  /** App theme. "system" follows the OS (`prefers-color-scheme`); "light"/
   *  "dark" force one regardless. `applyTheme` (lib/theme.ts) owns the `.dark`
   *  class on <html> + the OS-change listener when set to "system". */
  theme: "light" | "dark" | "system";
  /** Floating always-on-top partner widget (§D). Off by default — it's an
   *  opt-in miniature window pinned over other apps. The main window opens /
   *  closes it via services/widget.ts; the widget itself never opens a socket
   *  and receives all its state via the `harbor-widget-state` event. */
  widget_enabled: boolean;
  /** Play a short chime when a Harbor quick-message notification fires (Feature 5).
   *  Off by default — silent notifications are the Harbor default. When enabled,
   *  `useHarborNotifications` plays a bundled `/notif-chime.mp3` at low volume;
   *  the asset is optional (a missing file is a silent no-op). */
  notif_sound_enabled: boolean;
}

/**
 * The single built-in default relay URL — the one point that wires the
 * build-time production Worker URL into the client. Per the Cloudflare migration
 * (`harbor-cloud`), the relay moved off the local FastAPI process
 * (`ws://localhost:8000`) to a Cloudflare Worker. Dev runs the Worker locally via
 * `wrangler dev` on `:8787`; production is set at *build* time through
 * `import.meta.env.VITE_RELAY_URL` (Vite statically substitutes `VITE_*` keys),
 * e.g. `VITE_RELAY_URL=wss://harbor-cloud.<acct>.workers.dev npm run tauri build`.
 *
 * Every network path derives from this — `relay.ts:httpBase` builds HTTP from the
 * WS scheme, `ws.ts:toWsUrl` appends `/ws`, and `ensureIdentity` / Settings seed
 * from `Settings.relay_url` (which itself defaults to this). No per-call-site
 * env wiring: feed this central default only. Custom user URLs set in Settings
 * are persisted to `identity.json` and always take precedence at runtime.
 */
const PROD_RELAY_URL =
  typeof import.meta !== "undefined" && import.meta.env
    ? import.meta.env.VITE_RELAY_URL
    : undefined;
/**
 * Built-in default relay URL. The production Worker URL is NO LONGER hardcoded
 * here — it is injected at build time from a (local, non-versioned) `client/.env`
 * via the `VITE_RELAY_URL` key (Vite statically substitutes `VITE_*` keys). This
 * keeps your Cloudflare account handle out of the public repo.
 *
 * Fallback is the local `wrangler dev` default (`ws://localhost:8787`), which is
 * the standard first-run / development relay and is safe to version. Production
 * builds must set `VITE_RELAY_URL=wss://harbor-cloud.<acct>.workers.dev` in
 * `client/.env` (see `client/.env.example`). Custom user URLs set in Settings are
 * persisted to identity.json and always take precedence at runtime.
 */
export const DEFAULT_RELAY_URL: string =
  typeof PROD_RELAY_URL === "string" && PROD_RELAY_URL.length > 0
    ? PROD_RELAY_URL
    : "ws://localhost:8787";

export const DEFAULT_SETTINGS: Settings = {
  notify_on_online: true,
  notify_on_away: true,
  notify_on_offline: true,
  notify_on_message: true,
  away_after_minutes: 5,
  relay_url: DEFAULT_RELAY_URL,
  autostart_enabled: false,
  share_activity: true,
  theme: "system",
  widget_enabled: false,
  notif_sound_enabled: false,
};
