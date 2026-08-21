/**
 * supabase — thin, dependency-free Supabase (PostgREST) client for the Harbor
 * client. Used to persist the user's SETTINGS across devices/reinstalls (the
 * "configurações" the target architecture moves to Supabase). Everything here
 * is BEST-EFFORT and NON-BLOCKING: if Supabase is unreachable, unconfigured, or
 * the schema hasn't been applied yet, the app keeps working entirely on local
 * storage (the current behavior). No network call is ever on the critical path.
 *
 * Auth model: the anon (publishable) key + Row-Level Security. The `settings`
 * table is keyed by a `scope` — a stable key shared by BOTH devices in a pair
 * (derived from the two device_ids, sorted), so settings sync across the two
 * paired devices. RLS policies (see supabase/schema.sql) read the `X-Scope`
 * header the client sends and allow a device to read/write only its own scope.
 * The anon key is public by design — it is NOT a secret; the RLS policies are
 * what protect the data.
 *
 * We use plain `fetch` to the PostgREST REST API instead of the `@supabase/supabase-js`
 * package — one less dependency, smaller bundle, and the surface we need is tiny.
 */
import type { Identity, Settings } from "./types";

export const SUPABASE_URL = "https://ujtjnmaplgmhnxiohotz.supabase.co";
/** Publishable (anon) key — public by design, paired with RLS. */
export const SUPABASE_ANON_KEY = "sb_publishable_nCAy_f1fXzo5GeeX4qgOUw_cL3ZejTQ";

/** Is Supabase configured? Always true here (URL + key are baked in); kept as a
 *  single switch so a future build can disable it without touching call sites. */
export const SUPABASE_ENABLED = true;
const REMOTE_SETTINGS_CACHE_TTL_MS = 5 * 60 * 1000;
const remoteSettingsCache = new Map<string, { at: number; value: Partial<Settings> | null }>();
const remoteSettingsRequests = new Map<string, Promise<Partial<Settings> | null>>();

/**
 * Compute the settings sync scope for an identity.
 *
 *  - Paired: a stable key shared by BOTH devices in the pair, derived from the
 *    two device_ids (sorted). Both devices know their own device_id and their
 *    partner's (partner_id), so they compute the SAME scope → settings sync
 *    across the two paired devices (the "cross-device sync" goal).
 *  - Unpaired: the device's own id (per-device, no sync).
 */
export function settingsScope(id: Pick<Identity, "device_id" | "partner_id">): string {
  if (id.partner_id) {
    const [a, b] = [id.device_id, id.partner_id].sort();
    return `pair:${a}:${b}`;
  }
  return `dev:${id.device_id}`;
}

async function supabaseFetch(
  path: string,
  init: RequestInit = {},
  scope?: string,
): Promise<Response> {
  const res = await fetch(`${SUPABASE_URL}/rest/v1${path}`, {
    ...init,
    headers: {
      apikey: SUPABASE_ANON_KEY,
      Authorization: `Bearer ${SUPABASE_ANON_KEY}`,
      "Content-Type": "application/json",
      // RLS reads this header to allow access to the row for this scope.
      ...(scope ? { "X-Scope": scope } : {}),
      ...(init.headers ?? {}),
    },
  });
  if (!res.ok) throw new Error(`supabase ${path}: ${res.status}`);
  return res;
}

/** Load the settings row for a scope. Returns null if none exists yet. */
export async function loadRemoteSettings(
  scope: string,
): Promise<Partial<Settings> | null> {
  if (!SUPABASE_ENABLED) return null;
  const cached = remoteSettingsCache.get(scope);
  if (cached && Date.now() - cached.at < REMOTE_SETTINGS_CACHE_TTL_MS) return cached.value;
  const pending = remoteSettingsRequests.get(scope);
  if (pending) return pending;
  const request = (async () => {
    const res = await supabaseFetch(
      `/settings?scope=eq.${encodeURIComponent(scope)}&select=settings`,
      {},
      scope,
    );
    const rows = (await res.json()) as Array<{ settings: Partial<Settings> }>;
    const value = rows[0]?.settings ?? null;
    remoteSettingsCache.set(scope, { at: Date.now(), value });
    return value;
  })();
  remoteSettingsRequests.set(scope, request);
  try {
    return await request;
  } finally {
    if (remoteSettingsRequests.get(scope) === request) remoteSettingsRequests.delete(scope);
  }
}

/** Upsert the settings row for a scope (insert or replace on scope). */
export async function saveRemoteSettings(
  scope: string,
  settings: Settings,
): Promise<void> {
  if (!SUPABASE_ENABLED) return;
  await supabaseFetch(
    "/settings",
    {
      method: "POST",
      headers: { Prefer: "resolution=merge-duplicates" },
      body: JSON.stringify({
        scope,
        settings,
        updated_at: new Date().toISOString(),
      }),
    },
    scope,
  );
  remoteSettingsCache.set(scope, { at: Date.now(), value: settings });
}

/** Best-effort push of local settings to Supabase. Never throws to the caller —
 *  a failure is swallowed (the app keeps working on local storage). */
export function syncSettingsToRemote(scope: string, settings: Settings): void {
  if (!SUPABASE_ENABLED || !scope) return;
  void saveRemoteSettings(scope, settings).catch(() => {});
}

/** Best-effort merge of remote settings into local. Returns the merged settings
 *  (remote wins on defined keys) or the local ones if the remote is unavailable
 *  or has no row. Never throws. */
export async function mergeRemoteSettings(
  scope: string,
  local: Settings,
): Promise<Settings> {
  if (!SUPABASE_ENABLED || !scope) return local;
  try {
    const remote = await loadRemoteSettings(scope);
    if (!remote) return local;
    return { ...local, ...remote };
  } catch {
    return local;
  }
}
