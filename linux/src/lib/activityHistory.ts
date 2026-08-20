/**
 * Activity history persistence (SQLite `activity_history` table, migrated in
 * lib.rs v3). One row per observed foreground-app transition received from the
 * partner's `activity` event stream, derived by `ActivityTracker`
 * (activityDerivation.ts): an `open` when an app gains focus, a `switch` when
 * one app replaces another, and a `close` when the stream goes idle (null).
 *
 * Durations are computed from `started_at`/`ended_at` (ms epoch) and stored as
 * seconds (`duration_sec`) for cheap, human display ("Usou por 35 min").
 * Reuses `getDb()` from localDb.ts (same `sqlite:harbor.db` handle).
 */
import Database from "@tauri-apps/plugin-sql";
import { getDb } from "./localDb";

/** One derived transition row. */
export interface ActivityEvent {
  id: number;
  partner_id: string;
  /** Lowercased exe name ("chrome.exe"), or null for an idle `close`. */
  exe: string | null;
  /** "open" | "switch" | "close". */
  event: string;
  /** Epoch ms when the transition started. */
  started_at: number;
  /** Epoch ms when the transition ended (set on close/switch), else null. */
  ended_at: number | null;
  /** Whole seconds the app held focus, or null while still open. */
  duration_sec: number | null;
}

/** Raw shape returned by the sql plugin select. */
export interface ActivityEventRow {
  id: number;
  partner_id: string;
  exe: string | null;
  event: string;
  started_at: number;
  ended_at: number | null;
  duration_sec: number | null;
}

function rowToEvent(r: ActivityEventRow): ActivityEvent {
  return {
    id: r.id,
    partner_id: r.partner_id,
    exe: r.exe,
    event: r.event,
    started_at: r.started_at,
    ended_at: r.ended_at,
    duration_sec: r.duration_sec,
  };
}

/** Insert one derived transition. `endedAt` may be omitted (an open session). */
export async function insertActivityEvent(
  partnerId: string,
  exe: string | null,
  event: string,
  startedAt: number,
  endedAt?: number,
): Promise<void> {
  const conn = await getDb();
  const durationSec =
    endedAt != null ? Math.floor((endedAt - startedAt) / 1000) : null;
  await conn.execute(
    "INSERT INTO activity_history(partner_id, exe, event, started_at, ended_at, duration_sec) VALUES ($1,$2,$3,$4,$5,$6)",
    [partnerId, exe, event, startedAt, endedAt ?? null, durationSec],
  );
}

/** Load the partner's history, newest first (limit default 200). */
export async function loadActivityHistory(
  partnerId: string,
  limit = 200,
): Promise<ActivityEvent[]> {
  const conn = await getDb();
  const rows = await conn.select<ActivityEventRow[]>(
    "SELECT id, partner_id, exe, event, started_at, ended_at, duration_sec FROM activity_history WHERE partner_id = $1 ORDER BY started_at DESC LIMIT $2",
    [partnerId, limit],
  );
  return rows.map(rowToEvent);
}

/** Wipe all activity history — used on unpair (fresh slate for a new pair). */
export async function clearActivityHistory(): Promise<void> {
  const conn = await getDb();
  await conn.execute("DELETE FROM activity_history");
}

/** Re-export the shared handle for any direct callers. */
export { getDb, type Database };
