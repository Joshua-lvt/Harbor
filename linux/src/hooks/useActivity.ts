/**
 * Outbound foreground-app broadcasting + a local preview of MY current app.
 *
 * Polls the Rust `get_foreground_app` command every ~4s and, on change, sends
 * an `activity` envelope to the partner via the relay (forward-only,
 * transient). Harbor's own exe is filtered out so we never tell the partner
 * "using Harbor".
 *
 * `shareEnabled` gates the broadcast only — the local poll keeps running so the
 * user always sees a live preview of the activity that *would* be shared.
 * Toggling sharing off (while connected) sends a final `activity: null` so the
 * partner's side clears our stale app instead of freezing on the last one.
 * Toggling it back on re-advertises the current foreground app on the next tick.
 *
 * The RECEIVER side maps the exe → friendly name + game (lib/appNames.ts) and
 * shows it on Home + fires a toaster when a game starts.
 *
 * Feature 4 (real icons): `get_foreground_app` returns `{ exe, path }`. The
 * `activity` envelope still carries only the basename `exe` (privacy: the full
 * path never crosses the wire). On a transition to a NEW exe (while sharing),
 * we also push its icon ONCE per exe as `activity_icon` — the extraction is
 * async + non-blocking and NEVER gates the `activity` send. The icon travels
 * once; every subsequent `activity` for the same exe is just the lightweight
 * name. `activityPath` is exposed for the local self-row to render our OWN icon.
 */
import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { socket } from "../services/ws";
import { maybeSendActivityIcon } from "../lib/appIconCache";

const POLL_MS = 4000;

/** Mirror of the Rust `ForegroundApp` struct (icons.rs) — the exe basename is
 *  the activity key + icon-cache key; `path` is the full image-name used once
 *  per exe to extract the real icon. Either may be null on failure. */
interface ForegroundApp {
  exe: string | null;
  path: string | null;
}

async function foregroundApp(): Promise<ForegroundApp | null> {
  try {
    const app = await invoke<ForegroundApp | null>("get_foreground_app");
    return app ?? null;
  } catch {
    return null;
  }
}

export function useActivity(
  connected: boolean,
  shareEnabled: boolean,
): { activity: string | null; activityPath: string | null } {
  const [activity, setActivity] = useState<string | null>(null);
  const [activityPath, setActivityPath] = useState<string | null>(null);
  // Last value we actually broadcast — decoupled from the local display so we
  // can keep showing the app while sharing is off without spamming sends.
  const lastSent = useRef<string | null>(null);
  const lastShare = useRef(shareEnabled);
  // Exe names whose icon we've already extracted + pushed this session — the
  // once-per-exe guard for `maybeSendActivityIcon` (a Set is a stable ref so a
  // re-run of the poll effect doesn't reset it on every StrictMode remount).
  const sentIcons = useRef<Set<string>>(new Set());

  // Reflect sharing toggles immediately to the partner: turning OFF (while
  // connected) sends one `activity: null` so their view clears; turning ON
  // needs no explicit signal — the next poll re-advertises the current app.
  useEffect(() => {
    if (connected && lastShare.current && !shareEnabled) {
      socket.send({ type: "activity", app: null });
      lastSent.current = null;
    }
    lastShare.current = shareEnabled;
  }, [shareEnabled, connected]);

  // Poll my foreground app whenever connected, for BOTH the local preview and
  // the broadcast. Sharing only controls the send, not the poll.
  useEffect(() => {
    if (!connected) {
      setActivity(null);
      setActivityPath(null);
      lastSent.current = null;
      return;
    }
    let stopped = false;

    const tick = async () => {
      const app = await foregroundApp();
      if (stopped) return;
      let exe = app?.exe ?? null;
      const path = app?.path ?? null;
      // Don't broadcast Harbor's own window as an "activity".
      if (exe && /harbor/i.test(exe)) exe = null;
      setActivity(exe); // live local preview (shown in Home header)
      setActivityPath(exe ? path : null);
      if (shareEnabled && exe !== lastSent.current) {
        lastSent.current = exe;
        socket.send({ type: "activity", app: exe });
        // Feature 4: push this exe's icon ONCE to the partner. Non-blocking —
        // extract+send happens off the tick's critical path; a 4s loop only
        // pays the extraction cost the FIRST time a given app is seen.
        if (exe && path) maybeSendActivityIcon(exe, path, sentIcons.current);
      }
    };

    void tick();
    const h = window.setInterval(tick, POLL_MS);
    return () => {
      stopped = true;
      clearInterval(h);
      lastSent.current = null;
    };
  }, [connected, shareEnabled]);

  return { activity, activityPath };
}
