/**
 * Real app-icon cache + exchange (Feature 4).
 *
 * Two roles share one `app_icons` SQLite table (migrated v4 in lib.rs) keyed by
 * the lowercased exe name — the same key the `activity` event carries every 4s:
 *
 *  - SENDER (`maybeSendActivityIcon`, called from useActivity on a NEW exe):
 *      extracts its OWN foreground exe's icon via the Rust `get_app_icon`
 *      command (full path), caches it locally + sends it ONCE per exe over WS as
 *      `activity_icon`. Subsequent `activity` events keep carrying only the
 *      lightweight exe name; the icon payload goes once. Extraction is async +
 *      non-blocking — it never gates the `activity` send.
 *  - RECEIVER (`storePartnerIcon`, driven by the `activity_icon` WS handler in
 *      App.tsx): stores the partner's pushed icon into cache + SQLite. It does
 *      NOT extract the partner's exe from its own machine (the app may not be
 *      installed here) — it relies on the once-per-exe push.
 *
 * `getAppIcon(exePath, exeName)` is the single lookup the UI uses: memory →
 * SQLite → (only when an extraction PATH is available) `get_app_icon`. `null`
 * is a *meaningful* value — "confirmed no extractable icon" (→ the UI shows a
 * `GeneratedAppIcon` colored-circle fallback). `undefined` from the sync
 * `peekIcon` means "not yet resolved" (still loading).
 *
 * The `useAppIcon` hook makes this reactive: a component rendering the icon
 * re-resolves the instant a late `activity_icon` push warms the cache for its
 * exe, so an icon that arrives after first paint swaps in without a manual
 * re-render.
 */
import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getDb } from "./localDb";
import { friendlyName } from "./appNames";
import { socket } from "../services/ws";

/* ────────────────────────── in-memory cache ────────────────────────── */

/** exe (lowercased) → icon data URL (string) | confirmed-none (null).
 *  Absence = not yet resolved (peekIcon returns null for unstaged keys). */
const memory = new Map<string, string | null>();

/** exe (lowercased) → listeners fired when that exe's icon changes. */
const listeners = new Map<string, Set<(icon: string | null) => void>>();

const norm = (exeName: string): string => (exeName ?? "").toLowerCase();

function notify(key: string, icon: string | null): void {
  const set = listeners.get(key);
  if (set) for (const cb of set) cb(icon);
}

/** Subscribe to icon updates for one exe name. Fires immediately is NOT done
 *  here (the hook reads the current value itself); this only fires on a later
 *  cache change. Returns an unsubscribe. */
export function onIcon(
  exeName: string,
  cb: (icon: string | null) => void,
): () => void {
  const key = norm(exeName);
  let set = listeners.get(key);
  if (!set) {
    set = new Set();
    listeners.set(key, set);
  }
  set.add(cb);
  return () => {
    set!.delete(cb);
    if (set!.size === 0) listeners.delete(key);
  };
}

/** Sync memory lookup (for buildSnapshot, which can't await). null = no icon
 *  known yet OR confirmed-none — render sites treat both as "show fallback". */
export function peekIcon(exeName: string): string | null {
  return memory.get(norm(exeName)) ?? null;
}

/* ─────────────────────────── SQLite layer ──────────────────────────── */

interface AppIconRow {
  icon_data: string | null;
}

async function readIconFromDb(key: string): Promise<string | null | undefined> {
  const conn = await getDb();
  const rows = await conn.select<AppIconRow[]>(
    "SELECT icon_data FROM app_icons WHERE exe = $1",
    [key],
  );
  if (!rows.length) return undefined; // not stored
  return rows[0].icon_data;
}

/** UPSERT one icon row (string or confirmed-null) into `app_icons`. */
async function persistIcon(key: string, icon: string | null): Promise<void> {
  try {
    const conn = await getDb();
    await conn.execute(
      "INSERT OR REPLACE INTO app_icons(exe, icon_data, updated_at) VALUES ($1, $2, $3)",
      [key, icon, Date.now()],
    );
  } catch {
    // Non-fatal — the memory cache still works for this session; we just lose
    // the cold-start persistence of this one icon. Never break the UI over it.
  }
}

/* ─────────────────────────── public lookup ─────────────────────────── */

/**
 * Resolve an app icon by exe name, with an optional extraction PATH.
 *
 *  - `exePath` non-empty (sender / local self row): memory → SQLite → Rust
 *    `get_app_icon` → cache + persist. The extraction is what populates the
 *    cache for *this* device's own apps.
 *  - `exePath` empty (receiver): memory → SQLite only. The receiver never
 *    extracts the partner's exe (it may not be installed); it relies on the
 *    `activity_icon` push (`storePartnerIcon`). A miss returns null without
 *    persisting (the push will fill it + notify the hook).
 *
 * Always returns `string | null` (null → `GeneratedAppIcon` fallback).
 */
export async function getAppIcon(
  exePath: string,
  exeName: string,
): Promise<string | null> {
  const key = norm(exeName);
  if (!key) return null;

  // 1) memory hit (includes a confirmed-null).
  if (memory.has(key)) return memory.get(key) ?? null;

  // 2) SQLite hit.
  const stored = await readIconFromDb(key);
  if (stored !== undefined) {
    memory.set(key, stored);
    return stored;
  }

  // 3) No path → receiver-side miss: stage a transient null so the next call
  //    doesn't hit SQLite again, but DON'T persist (a later activity_icon push
  //    overrides it). Return null → fallback until the push lands.
  if (!exePath) {
    memory.set(key, null);
    return null;
  }

  // 4) Extract from our own machine (sender / self row) via the Rust command.
  let icon: string | null = null;
  try {
    icon = (await invoke<string | null>("get_app_icon", { exePath })) ?? null;
  } catch {
    icon = null; // command failed (non-Windows / bad path) → fallback
  }
  memory.set(key, icon);
  void persistIcon(key, icon); // fire-and-forget; notify now (memory is warm)
  notify(key, icon);
  return icon;
}

/* ─────────────────────────── sender side ──────────────────────────── */

/**
 * Send the icon for a foreground exe to the partner ONCE per exe (Feature 4).
 *
 * Called from `useActivity` on a transition to a NEW exe (while sharing). The
 * `sentSet` is the caller's per-session `Set<string>` — once an exe is in it,
 * we never re-extract or re-send, so a 4s poll loop only pays the extraction
 * cost the first time a given app is seen. Extraction is async + non-blocking;
 * the caller must never await it before sending the `activity` event.
 *
 * The icon we send is whatever `getAppIcon` resolves (string OR confirmed-null
 * — a null push tells the receiver "don't try, use the fallback", so it caches
 * null and stops looking). Both sides share the cache, so the sender's own UI
 * also picks the extracted icon up immediately for its self row.
 */
export function maybeSendActivityIcon(
  exeName: string,
  exePath: string,
  sentSet: Set<string>,
): void {
  const key = norm(exeName);
  if (!key || sentSet.has(key)) return;
  sentSet.add(key); // mark first — never retry a per-exe extraction this session
  // Non-blocking: extract + cache locally, then push. A failed extraction
  // still sends null once (confirmed fallback), which is informative too.
  void getAppIcon(exePath, exeName).then((icon) => {
    try {
      socket.send({ type: "activity_icon", app: key, icon });
    } catch {
      // socket may be closed mid-send; non-fatal — `activity` keeps flowing.
    }
  });
}

/* ─────────────────────────── receiver side ─────────────────────────── */

/**
 * Store an icon pushed by the partner (`activity_icon` WS event → App.tsx).
 * Writes memory + SQLite + notifies any `useAppIcon` mounted for this exe, so
 * a late-arriving icon swaps into already-rendered rows/cards without a manual
 * re-render. `icon` may be null (partner confirmed no extractable icon → we
 * cache null so our own lookups stop hunting + persist the fallback).
 */
export function storePartnerIcon(
  exeName: string,
  icon: string | null,
): void {
  const key = norm(exeName);
  if (!key) return;
  memory.set(key, icon);
  void persistIcon(key, icon);
  notify(key, icon);
}

/* ─────────────────────────── reactive hook ─────────────────────────── */

/**
 * Resolve an app icon reactively. Returns the current value (null = none /
 * loading → render `GeneratedAppIcon`) and re-resolves the instant a late
 * `activity_icon` push warms the cache for this exe — so an icon arriving
 * after first paint swaps in without the caller wiring any event itself.
 *
 * `exePath` is optional: pass the full path for the LOCAL self row (so the hook
 * can extract); omit it for the partner row (receiver-side, cache-only).
 */
export function useAppIcon(
  exeName: string,
  exePath?: string,
): string | null {
  const key = norm(exeName);
  const [icon, setIcon] = useState<string | null>(() => peekIcon(key));

  useEffect(() => {
    let cancelled = false;
    let cur = peekIcon(key);
    setIcon(cur);
    void getAppIcon(exePath ?? "", key).then((i) => {
      if (!cancelled && i !== cur) {
        cur = i;
        setIcon(i);
      }
    });
    const off = onIcon(key, (i) => {
      if (!cancelled && i !== cur) {
        cur = i;
        setIcon(i);
      }
    });
    return () => {
      cancelled = true;
      off();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key, exePath]);

  return icon;
}

/* ──────────────────────── generated fallback icon ─────────────────── */

/** Deterministic hue (0..359) from a string so each exe always gets the same
 *  color across renders + restarts. */
function hueOf(s: string): number {
  let h = 0;
  for (let i = 0; i < s.length; i++) h = (h * 31 + s.charCodeAt(i)) % 360;
  return h;
}

/** First displayable letter of the friendly name (e.g. "Google Chrome" → "G"),
 *  uppercased; "?" when the name is empty/blank. */
function initialOf(exe: string): string {
  const name = friendlyName(exe).trim();
  for (const ch of name) {
    if (/[A-Za-z0-9]/.test(ch)) return ch.toUpperCase();
  }
  return "?";
}

/**
 * A generated colored-circle + initial fallback, shown when no real icon is
 * cached for an exe (extraction failed / not yet pushed). Deterministic color
 * from the exe name so the same app is always the same color across renders.
 * Purely presentational — same sizing contract as a real `<img>` icon.
 */
export function GeneratedAppIcon({
  exe,
  size,
  className,
}: {
  exe: string;
  size: number;
  className?: string;
}) {
  const hue = hueOf(exe);
  const fs = Math.max(10, Math.floor(size * 0.5));
  return (
    <span
      className={`inline-flex shrink-0 items-center justify-center rounded-md ${className ?? ""}`}
      style={{
        width: size,
        height: size,
        background: `hsl(${hue} 55% 55%)`,
        color: "white",
        fontSize: fs,
        fontWeight: 700,
        lineHeight: 1,
      }}
      aria-hidden
    >
      {initialOf(exe)}
    </span>
  );
}

export default GeneratedAppIcon;
