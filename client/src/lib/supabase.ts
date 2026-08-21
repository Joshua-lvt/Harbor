/**
 * supabase — thin, dependency-free Supabase (PostgREST) client for the Harbor
 * client. Used to persist the user's SETTINGS across devices/reinstalls (the
 * "configurações" the target architecture moves to Supabase). Everything here
 * is BEST-EFFORT and NON-BLOCKING: if Supabase is unreachable, unconfigured, or
 * the schema hasn't been applied yet, the app keeps working entirely on local
 * storage (the current behavior). No network call is ever on the critical path.
 *
 * Auth model: the anon (publishable) key + Row-Level Security. The `settings`
 * table is keyed by `device_id`; RLS policies (see supabase/schema.sql) let a
 * device read/write only its own row. The anon key is public by design — it is
 * NOT a secret; the RLS policies are what protect the data.
 *
 * We use plain `fetch` to the PostgREST REST API instead of the `@supabase/supabase-js`
 * package — one less dependency, smaller bundle, and the surface we need is tiny.
 */
import type { Settings } from "./types";

export const SUPABASE_URL = "https://ujtjnmaplgmhnxiohotz.supabase.co";
/** Publishable (anon) key — public by design, paired with RLS. */
export const SUPABASE_ANON_KEY = "sb_publishable_nCAy_f1fXzo5GeeX4qgOUw_cL3ZejTQ";

/** Is Supabase configured? Always true here (URL + key are baked in); kept as a
 *  single switch so a future build can disable it without touching call sites. */
export const SUPABASE_ENABLED = true;

async function supabaseFetch(
  path: string,
  init: RequestInit = {},
  deviceId?: string,
): Promise<Response> {
  const res = await fetch(`${SUPABASE_URL}/rest/v1${path}`, {
    ...init,
    headers: {
      apikey: SUPABASE_ANON_KEY,
      Authorization: `Bearer ${SUPABASE_ANON_KEY}`,
      "Content-Type": "application/json",

      ...(deviceId
        ? {
            "X-Device-Id": deviceId,
          }
        : {}),

      ...(init.headers ?? {}),
    },
  });

  if (!res.ok) throw new Error(`supabase ${path}: ${res.status}`);
  return res;
}

/** Load the settings row for a device. Returns null if none exists yet. */
export async function loadRemoteSettings(
  deviceId: string,
): Promise<Partial<Settings> | null> {
  if (!SUPABASE_ENABLED) return null;
  const res = await supabaseFetch(
  `/settings?device_id=eq.${encodeURIComponent(deviceId)}&select=settings`,
  {},
  deviceId,
);
  const rows = (await res.json()) as Array<{ settings: Partial<Settings> }>;
  return rows[0]?.settings ?? null;
}

/** Upsert the settings row for a device (insert or replace on device_id). */
export async function saveRemoteSettings(
  deviceId: string,
  settings: Settings,
): Promise<void> {
  if (!SUPABASE_ENABLED) return;
  await supabaseFetch(
  "/settings",
  {
    method: "POST",
    headers: { Prefer: "resolution=merge-duplicates" },
    body: JSON.stringify({
      device_id: deviceId,
      settings,
      updated_at: new Date().toISOString(),
    }),
  },
  deviceId,
);
}

/** Best-effort push of local settings to Supabase. Never throws to the caller —
 *  a failure is swallowed (the app keeps working on local storage). */
export function syncSettingsToRemote(deviceId: string, settings: Settings): void {
  if (!SUPABASE_ENABLED || !deviceId) return;
  void saveRemoteSettings(deviceId, settings).catch(() => {});
}

/** Best-effort merge of remote settings into local. Returns the merged settings
 *  (remote wins on defined keys) or the local ones if the remote is unavailable
 *  or has no row. Never throws. */
export async function mergeRemoteSettings(
  deviceId: string,
  local: Settings,
): Promise<Settings> {
  if (!SUPABASE_ENABLED || !deviceId) return local;
  try {
    const remote = await loadRemoteSettings(deviceId);
    if (!remote) return local;
    return { ...local, ...remote };
  } catch {
    return local;
  }
}
