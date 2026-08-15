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

/** Hook into server events and emit the four Harbor notification kinds. */
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
  return subscribe((e) => {
    if (e.type === "presence") {
      if (e.state === lastPresence) return; // dedup — only notify on a change
      lastPresence = e.state;
      if (e.state === "online") notify("💙 Harbor", `${name} ficou online.`, s, "notify_on_online");
      else if (e.state === "away") notify("🌙 Harbor", `${name} ficou ausente.`, s, "notify_on_away");
      else if (e.state === "offline")
        notify("⚫ Harbor", `${name} ficou offline.`, s, "notify_on_offline");
    } else if (e.type === "chat") {
      notify("💬 Harbor", "Nova mensagem recebida.", s, "notify_on_message");
    }
  });
}
