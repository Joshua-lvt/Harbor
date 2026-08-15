/**
 * Device identity + settings persistence, backed by @tauri-apps/plugin-store.
 *
 * Two stores:
 *  - identity.json : device_id, device_secret, pairing_code, relay_url, partner.
 *  - settings.json : notification toggles, away timeout, autostart flag.
 *
 * `relay_url` lives in identity so the WS client reads one source of truth.
 */
import { LazyStore } from "@tauri-apps/plugin-store";
import type { Identity, Settings } from "./types";
import { DEFAULT_RELAY_URL, DEFAULT_SETTINGS } from "./types";

const identityStore = new LazyStore("identity.json");
const settingsStore = new LazyStore("settings.json");

/**
 * Prior built-in relay defaults. An install that still points at one of these was
 * never customized by the user (the Settings field seeded from the same default),
 * so we auto-rewrite it to the CURRENT built-in default on load — a one-time
 * migration to the Cloudflare backend that needs no manual Settings edit on either
 * device. A URL that does NOT match a known default is a real user override and is
 * preserved untouched (Settings still lets the user override anytime).
 *
 * Two entries cover the two migration stages:
 *   - `ws://localhost:8000`  — the pre-migration FastAPI relay default
 *   - `ws://localhost:8787`  — the wrangler-dev default after the migration
 *
 * Rewriting `8787` → DEFAULT is what carries an install from the dev stage to the
 * production stage on a prod build (where `DEFAULT_RELAY_URL = wss://<prod>`). The
 * `migrated url !== DEFAULT_RELAY_URL` guard makes the rewrite a no-op once the
 * install is already on the current default (so it won't fight a real `8787`
 * customization made by someone developing against local wrangler — they can
 * restore it in Settings; this is an acknowledged trade-off for a 2-device app
 * whose two users are also the operator).
 */
const LEGACY_DEFAULT_RELAY_URLS: readonly string[] = [
  "ws://localhost:8000",
  "ws://localhost:8787",
];

/**
 * Return the relay URL a persisted record should use, migrating a known stale
 * default to the current default. A non-legacy URL (a real user override, or
 * already the current default) is returned unchanged, which lets callers guard
 * the one-time write with `migrated !== stored`.
 */
function migrateRelayUrl(url: string | null | undefined): string {
  if (typeof url === "string" && !LEGACY_DEFAULT_RELAY_URLS.includes(url)) return url;
  return DEFAULT_RELAY_URL;
}

/** Generate a UUID v4. Uses crypto.randomUUID when available (Tauri WebView2 has it). */
export function newDeviceId(): string {
  if (typeof crypto !== "undefined" && "randomUUID" in crypto) {
    return crypto.randomUUID();
  }
  // Fallback (shouldn't run in WebView2).
  return "xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx".replace(/[xy]/g, (c) => {
    const r = (Math.random() * 16) | 0;
    const v = c === "x" ? r : (r & 0x3) | 0x8;
    return v.toString(16);
  });
}

export async function loadIdentity(): Promise<Identity | null> {
  const raw = await identityStore.get<Identity>("identity");
  if (!raw) return null;
  // One-time existing-install migration: an identity still on a known stale relay
  // default is rewritten to the current default. `migrateRelayUrl` leaves a real
  // user override untouched, so the `!== stored` guard makes the write fire once
  // (and never when already current).
  const newUrl = migrateRelayUrl(raw.relay_url);
  if (newUrl !== raw.relay_url) {
    const migrated: Identity = { ...raw, relay_url: newUrl };
    await identityStore.set("identity", migrated);
    await identityStore.save();
    return migrated;
  }
  return raw;
}

export async function saveIdentity(id: Identity): Promise<void> {
  await identityStore.set("identity", id);
  await identityStore.save();
}

/** Create a fresh device identity (first launch or after a wipe). */
export async function ensureIdentity(defaultRelayUrl: string): Promise<Identity> {
  const existing = await loadIdentity();
  if (existing && existing.device_id && existing.device_secret) {
    return existing;
  }
  const fresh: Identity = {
    device_id: newDeviceId(),
    device_secret: "",
    pairing_code: null,
    relay_url: defaultRelayUrl,
    partner_id: null,
    partner_name: null,
    my_name: null,
  };
  await saveIdentity(fresh);
  return fresh;
}

export async function loadSettings(defaults: Settings = DEFAULT_SETTINGS): Promise<Settings> {
  const raw = await settingsStore.get<Partial<Settings>>("settings");
  const merged: Settings = { ...defaults, ...(raw ?? {}) };
  // Mirror of the identity migration: an install whose persisted Settings relay
  // URL is a known stale default moves to the current default (on a prod build,
  // that carries dev/wrangler installs to the production Worker). Custom URLs —
  // anything else — are preserved. `!== stored` makes the write fire once.
  const newUrl = migrateRelayUrl(merged.relay_url);
  if (newUrl !== merged.relay_url) {
    const migrated: Settings = { ...merged, relay_url: newUrl };
    await saveSettings(migrated);
    return migrated;
  }
  return merged;
}

export async function saveSettings(s: Settings): Promise<void> {
  await settingsStore.set("settings", s);
  await settingsStore.save();
}
