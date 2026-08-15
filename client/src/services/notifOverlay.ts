/**
 * The always-on-top notification overlay — a second, frameless transparent
 * Tauri window created at runtime (NOT declared in tauri.conf.json, so it only
 * exists while a notification is showing). Mirrors the widget
 * (`services/widget.ts`) in shape: idempotent create/close, label-keyed, no
 * second socket (one-socket-per-device — a second connection would evict the
 * main window's).
 *
 * The overlay renders `NotificationOverlay.tsx` at `#/notif-overlay`. It listens
 * for `harbor-notification` events (broadcast by `emitNotification` below) and
 * never touches the socket, store, or relay. It emits `harbor-navigate` so the
 * main window can focus itself + route to chat on "Responder" / "Abrir chat".
 * When the queue empties, the main window closes the overlay via `closeNotifOverlay`.
 */
import { WebviewWindow } from "@tauri-apps/api/webviewWindow";
import { emit } from "@tauri-apps/api/event";

export const NOTIF_OVERLAY_LABEL = "notif-overlay";
const NOTIF_EVENT = "harbor-notification";
export const NOTIF_NAVIGATE_EVENT = "harbor-navigate";
export const NOTIF_DISMISS_EVENT = "harbor-notif-dismiss";

/** Re-export `emit` so the overlay (which renders in a separate JS realm) can
 *  broadcast `harbor-navigate` / `harbor-notif-dismiss` without re-importing the
 *  Tauri event module directly — keeps the overlay's surface to this module. */
export { emit } from "@tauri-apps/api/event";

export interface NotifPayload {
  id: string;
  partnerName: string;
  partnerAvatar: string | null;
  text: string;
  presetId?: string;
  timestamp: number;
  repeatCount: number;
  /** Whether to render the overlay card in dark mode. Resolved by the main
   *  window (the overlay has no Settings store permission); ignored in the
   *  Toaster path. */
  dark?: boolean;
}

/** Create the overlay window if it doesn't already exist. Idempotent on label.
 *  The window is frameless + transparent + always-on-top + skip taskbar, sized
 *  to one notification card. Errors are swallowed — the overlay is opt-in and
 *  must never break the app (a missing capability degrades to in-app toast only). */
export async function openNotifOverlay(): Promise<WebviewWindow | null> {
  const existing = await WebviewWindow.getByLabel(NOTIF_OVERLAY_LABEL);
  if (existing) return existing;
  try {
    return new WebviewWindow(NOTIF_OVERLAY_LABEL, {
      url: "index.html#/notif-overlay",
      title: "Harbor",
      width: 360,
      height: 130,
      decorations: false,
      resizable: false,
      skipTaskbar: true,
      alwaysOnTop: true,
      transparent: true,
      visible: true,
    });
  } catch (e) {
    console.warn("notif-overlay: could not create window", e);
    return null;
  }
}

/** Close + destroy the overlay window if it exists. Swallows errors. */
export async function closeNotifOverlay(): Promise<void> {
  try {
    const w = await WebviewWindow.getByLabel(NOTIF_OVERLAY_LABEL);
    await w?.close();
  } catch (e) {
    console.warn("notif-overlay: could not close window", e);
  }
}

/** True if the overlay window currently exists. */
export async function notifOverlayExists(): Promise<boolean> {
  try {
    const w = await WebviewWindow.getByLabel(NOTIF_OVERLAY_LABEL);
    return !!w;
  } catch {
    return false;
  }
}

/** Broadcast one notification to the overlay. Global emit (not emitTo) so a
 *  freshly-created window also receives the next one. Swallows errors. */
export async function emitNotification(payload: NotifPayload): Promise<void> {
  try {
    await emit(NOTIF_EVENT, payload);
  } catch (e) {
    console.warn("notif-overlay: could not emit notification", e);
  }
}
