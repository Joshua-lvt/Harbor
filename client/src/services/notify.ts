/**
 * Native Windows notifications via @tauri-apps/plugin-notification.
 *
 * Note from the Tauri docs: notifications in *dev* builds show with the
 * PowerShell name/icon — only installed apps show their own. That's expected
 * and resolves once the NSIS installer is built.
 */
import {
  isPermissionGranted,
  requestPermission,
  sendNotification,
} from "@tauri-apps/plugin-notification";
import { asMs } from "../lib/localDb";
import { notificationStore } from "./notificationStore";
import type { Settings } from "../lib/types";

let permissionGranted: boolean | null = null;

async function ensurePermission(): Promise<boolean> {
  if (permissionGranted === true) return true;
  if (permissionGranted === null) {
    let granted = await isPermissionGranted();
    if (!granted) {
      const perm = await requestPermission();
      granted = perm === "granted";
    }
    permissionGranted = granted;
  }
  return permissionGranted;
}

export async function notify(
  title: string,
  body: string,
  s: Settings,
  key: keyof Settings,
): Promise<void> {
  if (!s[key]) return; // toggle off
  if (!(await ensurePermission())) return;
  sendNotification({ title, body });
}

/** Hook into server events and emit the presence Harbor notification kinds.
 *
 * NOTE: real chat-message ("Nova mensagem recebida") notifications are NOT
 * fired here. This subscriber sees the raw wire `chat` event, which under E2E
 * is opaque ciphertext — it can't tell a real message from a Harbor
 * quick-message (`harbor_notify`) until it's decrypted. Firing here would mean
 * an OS toast for EVERY `chat` event, including quick-messages that are
 * supposed to surface only as the giant overlay (never the OS notification).
 * The chat-message OS toast now fires from the `messages` singleton — AFTER
 * decrypt, so it correctly skips quick-messages (which are routed to the
 * notification queue before the insert path).
 */
/** How long a presence state must persist before we notify on it. Absorbs the
 *  reconnect-induced flapping (the server echoes `online` on every socket open,
 *  then the client re-asserts `away`) and any residual idle-threshold jitter —
 *  the "ficou online/ausente" spam. A state that reverts within the window is
 *  never notified. */
const PRESENCE_GRACE_MS = 30_000;

export function attachNotifications(
  subscribe: (h: (e: import("../lib/types").ServerEvent) => void) => () => void,
  s: Settings,
  partnerName: string | null,
): () => void {
  const name = partnerName || "Seu parceiro";
  // Track the last seen presence so we only notify on a REAL transition, not on
  // every redundant `online` the server echoes on each WS (re)connect. Without
  // this, a brief network blip or the partner's reconnect spams "ficou online"
  // repeatedly (the server forwards `online` on every socket open, and the
  // client re-asserts its own presence on every onopen). Undefined = "never
  // seen", so the very first `online` after app start still fires.
  let lastPresence: "online" | "away" | "offline" | undefined;
  // Debounce timer for the presence toast (see PRESENCE_GRACE_MS).
  let pendingTimer: number | null = null;
  let pendingState: "online" | "away" | "offline" | null = null;

  function firePresence(state: "online" | "away" | "offline"): void {
    if (state === "online") notify("💙 Harbor", `${name} ficou online.`, s, "notify_on_online");
    else if (state === "away") notify("🌙 Harbor", `${name} ficou ausente.`, s, "notify_on_away");
    else if (state === "offline")
      notify("⚫ Harbor", `${name} ficou offline.`, s, "notify_on_offline");
  }

  return subscribe((e) => {
    if (e.type !== "presence") return;
    if (e.state === lastPresence) return; // dedup — only notify on a change
    lastPresence = e.state;
    // Record to the in-app Harbor notification history (Bug 4) regardless of
    // focus — the Notifications tab always shows the presence trail.
    void notificationStore.add({
      id: `presence-${e.device_id}-${e.ts}`,
      kind: "presence",
      title: name,
      body:
        e.state === "online"
          ? "Ficou online."
          : e.state === "away"
            ? "Ficou ausente."
            : "Ficou offline.",
      icon: e.state === "online" ? "💙" : e.state === "away" ? "🌙" : "⚫",
      timestamp: asMs(e.ts),
    });
    // Debounce the toast: only fire if the state persists for the grace window.
    // A transient flip (reconnect echo, idle jitter) cancels the pending toast.
    if (pendingTimer != null) clearTimeout(pendingTimer);
    pendingState = e.state;
    pendingTimer = window.setTimeout(() => {
      pendingTimer = null;
      if (pendingState == null) return;
      // Always fire the OS notification (when the toggle is on). The old code
      // gated on document.hasFocus() to show an in-app toast instead when the
      // app was "focused" — but on Linux a close-to-tray app reports focus even
      // in the background, so the OS notification NEVER fired (only the in-app
      // one). The in-app notification HISTORY (Notifications tab) is recorded
      // above regardless; the OS toast is the alert the user actually wants.
      firePresence(pendingState);
    }, PRESENCE_GRACE_MS);
  });
}
