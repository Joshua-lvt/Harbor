/**
 * The always-on-top partner widget — a second, minimal Tauri window created at
 * runtime (NOT declared in tauri.conf.json, so it only exists while enabled).
 *
 * Two-window, ONE-socket rule: the widget never opens its own WebSocket. The
 * relay's `ConnectionManager` maps a device_id to a single live socket, so a
 * second connection would evict the main window's socket (presence + chat +
 * voice would all drop). Instead the main window owns the socket and PUSHES a
 * snapshot of everything the widget needs via a global Tauri event
 * (`harbor-widget-state`). The widget only listens + renders.
 *
 * `openWidget()` creates the window once (idempotent on label); `closeWidget()`
 * closes + destroys it; `emitWidgetState(snap)` broadcasts a fresh snapshot.
 * The widget's ✕ button emits `harbor-widget-closed`; the main window listens
 * for it and flips `settings.widget_enabled` to false (Settings stays the single
 * source of truth — the widget never writes the store).
 */
import { WebviewWindow } from "@tauri-apps/api/webviewWindow";
import { emit } from "@tauri-apps/api/event";
import type { VoiceStatus } from "./voice";
import type { Customization } from "../lib/customization";

/** Label of the widget window. Also the hash route is keyed off it elsewhere. */
export const WIDGET_LABEL = "widget";

/** Everything the widget paints, bundled so the main window can emit one event
 *  per change instead of N partial ones. Mirrors the app-lifetime state already
 *  living in App.tsx (presence, activity, voice, identity, theme). */
export interface WidgetSnapshot {
  connected: boolean;
  partnerPresence: "online" | "away" | "offline";
  /** Raw lowercased exe of the partner's current foreground app, or null. */
  partnerActivity: string | null;
  /** Feature 4: the partner's cached app icon for `partnerActivity`
   *  (cache-only lookup by the main window). null → the widget renders a
   *  `GeneratedAppIcon` fallback. Updated on every activity_icon push that
   *  targets the current activity (the reconciler re-emits on iconTick). */
  partnerActivityIcon: string | null;
  partnerName: string;
  partnerAvatar: string | null;
  myName: string;
  myAvatar: string | null;
  /** My own current foreground-app exe (for the optional self row / future). */
  myActivity: string | null;
  /** Mirrors the `voice` singleton's status (see services/voice.ts). The widget
   *  doesn't render this today, but the main window emits it so a future widget
   *  could — and the type stays aligned with the source so App.tsx's
   *  `voiceState.status` assignment typechecks. */
  voiceStatus: VoiceStatus;
  /** Whether `.dark` is currently applied — applied to the widget's <html>. */
  dark: boolean;
  /** Personalization (Personalização) — the widget is a separate JS realm with
   *  no store, so its custom `--color-harbor-*` palette + bg-solid + hide-mascot
   *  class toggles arrive here and are applied by `WidgetScreen` via
   *  `applyCustomization`. Defaults to ocean when the main window hasn't emitted. */
  customization: Customization;
}

const STATE_EVENT = "harbor-widget-state";
export const WIDGET_CLOSED_EVENT = "harbor-widget-closed";
/** Handshake the widget fires once after its `harbor-widget-state` listener is
 *  registered (WidgetScreen mounts). The main window re-emits the snapshot on
 *  this signal — closes the boot race where the main window's first `emitNow`
 *  (right after `openWidget`) fired before the freshly-created second window
 *  had subscribed, leaving it stranded on the placeholder until a manual
 *  toggle re-opened it. With the handshake, the widget requests its initial
 *  state regardless of which side mounts first. */
export const WIDGET_READY_EVENT = "harbor-widget-ready";

/** Create the widget window if it doesn't already exist. The constructor
 *  options carry `alwaysOnTop` / `decorations` / `skipTaskbar` / `resizable`
 *  / `minWidth` / `minHeight` directly, so no runtime window commands are
 *  needed for those. `resizable: true` alone wouldn't help (the window is
 *  frameless + transparent — no OS resize border to grab); the actual resize
 *  surface is the corner grip in WidgetScreen that calls
 *  `startResizeDragging('SouthEast')`. Returns the window (existing or new).
 *  Errors (e.g. permissions missing) are swallowed + logged — the widget is
 *  opt-in and must never break the main app. */
export async function openWidget(): Promise<WebviewWindow | null> {
  // `WebviewWindow.getByLabel` returns null when absent — avoid recreating it,
  // which would race two windows on the same label.
  const existing = await WebviewWindow.getByLabel(WIDGET_LABEL);
  if (existing) return existing;
  try {
    return new WebviewWindow(WIDGET_LABEL, {
      url: "index.html#/widget",
      title: "Harbor",
      width: 240,
      height: 104,
      minWidth: 180,
      minHeight: 88,
      decorations: false,
      // Resizable via a corner grip (see WidgetScreen's ↘ handle, which calls
      // `startResizeDragging('SouthEast')`). The window is frameless + transparent
      // so the OS provides no resize border to grab — the grip is the only way in.
      resizable: true,
      skipTaskbar: true,
      alwaysOnTop: true,
      transparent: true,
      visible: true,
    });
  } catch (e) {
    console.warn("widget: could not create window", e);
    return null;
  }
}

/** Close + drop the widget window if it exists. Swallows errors (the toggle
 *  flips to off regardless — worst case the window was already gone). */
export async function closeWidget(): Promise<void> {
  try {
    const w = await WebviewWindow.getByLabel(WIDGET_LABEL);
    await w?.close();
  } catch (e) {
    console.warn("widget: could not close window", e);
  }
}

/** Returns true if the widget window currently exists (used by the main window's
 *  lifecycle effect to decide whether to (re)create vs. re-emit state). */
export async function widgetExists(): Promise<boolean> {
  try {
    const w = await WebviewWindow.getByLabel(WIDGET_LABEL);
    return !!w;
  } catch {
    return false;
  }
}

/** Broadcast a fresh snapshot to all windows. The widget listens for this event
 *  and re-renders. Used by the main window's reconciler effect whenever any
 *  snapshot input changes. Global (not emitTo) so a freshly-created widget also
 *  receives the next emit without the caller vouching for its readiness. */
export async function emitWidgetState(snap: WidgetSnapshot): Promise<void> {
  try {
    await emit(STATE_EVENT, snap);
  } catch (e) {
    console.warn("widget: could not emit state", e);
  }
}

/** Emit the "the user closed the widget via its ✕" event. The main window
 *  listens for it and flips `widget_enabled` to false (Settings is the single
 *  source of truth — the widget itself has no store permission to write). */
export async function emitWidgetClosed(): Promise<void> {
  try {
    await emit(WIDGET_CLOSED_EVENT);
  } catch (e) {
    console.warn("widget: could not emit closed", e);
  }
}
