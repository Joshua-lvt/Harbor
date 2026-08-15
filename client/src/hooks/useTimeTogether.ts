/**
 * "Juntos hoje" — cumulative seconds BOTH devices were online TOGETHER today,
 * persisted per-day in localStorage. Powers the "Juntos hoje" card on Home.
 *
 * "Together" is defined locally as: our socket is open (`connected`) AND the
 * partner's presence (delivered over the socket) is `online`. The relay is the
 * source of truth for the partner's presence, so this honors real disconnections
 * / privacy choices (if either side is off, the clock doesn't run).
 *
 * Sampling (30s, not 1s): the card displays a coarse "há X min / Xh Ymin"
 * label and re-renders every minute from Home; a 30s accumulator is plenty
 * precise for that and costs nothing. Half-minute granularity also keeps the
 * store tiny and avoids burning writes on every poll.
 *
 * Rollover at local midnight: each local date has its own entry; on the first
 * sample of a new day we seed `0` for the new key, so we never add to
 * yesterday's total. Old entries (KEEP_DAYS ago) are pruned when we save, so
 * the store stays flat.
 */
import { useEffect, useRef, useState } from "react";
import type { PresenceState } from "../lib/types";

const KEY = "harbor:together-time";
const SAMPLE_MS = 30_000; // 30-second accumulation granularity
const KEEP_DAYS = 14; // prune entries older than this so the store stays flat

interface TogetherLog {
  /** ISO local date "YYYY-MM-DD" → seconds together that day. Guessed-shape: an
   *  old store with a top-level number (a pre-keyed "today" total) is migrated
   *  transparently on first read; once a keyed log is written we never look
   *  back. */
  days: Record<string, number>;
}

/** Local-date key (NOT UTC) — "today" is the user's today. */
function todayKey(d = new Date()): string {
  const y = d.getFullYear();
  const m = String(d.getMonth() + 1).padStart(2, "0");
  const day = String(d.getDate()).padStart(2, "0");
  return `${y}-${m}-${day}`;
}

function load(): TogetherLog {
  try {
    const raw = localStorage.getItem(KEY);
    if (!raw) return { days: {} };
    const parsed = JSON.parse(raw);
    // Accept a keyed log; tolerate a stray old numeric total by migrating it
    // into today's bucket (one-time).
    if (parsed && typeof parsed === "object" && parsed.days) return parsed as TogetherLog;
    if (typeof parsed === "number") return { days: { [todayKey()]: parsed } };
    return { days: {} };
  } catch {
    return { days: {} };
  }
}

function save(log: TogetherLog): void {
  try {
    // Prune entries older than KEEP_DAYS days so the store stays flat.
    const cutoff = new Date();
    cutoff.setDate(cutoff.getDate() - KEEP_DAYS);
    const cutoffKey = todayKey(cutoff);
    const pruned: Record<string, number> = {};
    for (const [k, v] of Object.entries(log.days)) {
      if (k >= cutoffKey) pruned[k] = v;
    }
    localStorage.setItem(KEY, JSON.stringify({ days: pruned }));
  } catch {
    // localStorage unavailable / quota — the in-memory accumulator still works
  }
}

/** Format the accumulated seconds as a coarse duration label. */
export function togetherLabel(secs: number): string {
  if (secs < 60) return "menos de 1 min";
  const m = Math.floor(secs / 60);
  if (m < 60) return `${m} min`;
  const h = Math.floor(m / 60);
  const remMin = m % 60;
  return remMin === 0 ? `${h} h` : `${h} h ${remMin} min`;
}

/** Returns total seconds together TODAY (live, re-renders on each sample). */
export function useTimeTogether(
  connected: boolean,
  partnerPresence: PresenceState,
): number {
  const [secondsToday, setSecondsToday] = useState<number>(
    () => load().days[todayKey()] ?? 0,
  );
  const accumRef = useRef<number>(secondsToday);

  // The accumulator: ticks every SAMPLE_MS while we're together. Strict gating —
  // if either side drops (socket closed or partner not online) the clock stops.
  useEffect(() => {
    const bothOnline = connected && partnerPresence === "online";
    if (!bothOnline) return;

    const tick = () => {
      // Rollover at local midnight — if the day changed mid-session, seed 0 for
      // the new day so we don't add to yesterday's total.
      const log = load();
      const key = todayKey();
      if (!(key in log.days)) log.days[key] = 0;
      log.days[key] += Math.round(SAMPLE_MS / 1000);
      accumRef.current = log.days[key];
      save(log);
      setSecondsToday(accumRef.current);
    };
    // Wait a full SAMPLE_MS before the first accumulation (we credit time we
    // actually stayed together, not "right now"), then keep ticking.
    const h = window.setInterval(tick, SAMPLE_MS);
    return () => clearInterval(h);
  }, [connected, partnerPresence]);

  return secondsToday;
}
