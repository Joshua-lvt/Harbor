/**
 * useHarborNotifications — mounted ONCE in App for the app lifetime. Decides
 * which surface a quick-message notification shows on (Feature 5):
 *
 *  - main window focused + visible → in-app `Toaster` (the existing frosted
 *    toast at the top-right of the app window).
 *  - otherwise (minimized / unfocused) → the separate always-on-top
 *    `notif-overlay` Tauri window (built like the widget), so Harbor's own
 *    notification slides in over whatever the user is doing.
 *
 * Quick-messages are E2E-encrypted and notification-only (detected in useChat's
 * `harbor_notify` inner kind → `notificationQueue.enqueue`). This hook drives
 * the display-slot rotation: each enqueued card shows for NOTIF_DISPLAY_MS, then
 * the front is shifted so the next card surfaces. Collapses (identical preset
 * within 5s) update the live card in place and reset the timer. When the queue
 * drains the overlay window is closed.
 *
 * The overlay is a separate Tauri window = a separate JS realm, so the queue
 * singleton can't be shared; each new notification is broadcast to the overlay
 * via `emitNotification` (a global Tauri event), and dismissed-via-overlay is
 * honored through the `harbor-notif-dismiss` event the overlay emits.
 */
import { useEffect, useRef } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { listen } from "@tauri-apps/api/event";
import { notificationQueue, NOTIF_DISPLAY_MS } from "../services/notificationQueue";
import {
  openNotifOverlay,
  closeNotifOverlay,
  emitNotification,
  type NotifPayload,
  NOTIF_DISMISS_EVENT,
} from "../services/notifOverlay";
import { useToast } from "../components/Toaster";
import type { Settings } from "../lib/types";

function resolvedDark(theme: Settings["theme"]): boolean {
  return (
    theme === "dark" ||
    (theme === "system" &&
      window.matchMedia("(prefers-color-scheme: dark)").matches)
  );
}

export function useHarborNotifications(settings: Settings): void {
  const { push } = useToast();
  const pushRef = useRef(push);
  useEffect(() => {
    pushRef.current = push;
  }, [push]);

  useEffect(() => {
    let timer: number | null = null;

    async function isFocusedAndVisible(): Promise<boolean> {
      try {
        const w = getCurrentWindow();
        const [focused, visible] = await Promise.all([
          w.isFocused(),
          w.isVisible(),
        ]);
        return focused && visible;
      } catch {
        // Non-Tauri / WebView fallback: treat as focused so the in-app toast is
        // the surface (the overlay needs Tauri's window API anyway).
        return true;
      }
    }

    /** Surface one notification on the right medium for RIGHT NOW. */
    async function surface(n: NotifPayload, dark: boolean) {
      if (await isFocusedAndVisible()) {
        // In-app toast — the Toaster owns its own auto-dismiss.
        pushRef.current({
          icon: "💙",
          title: n.partnerName?.trim() || "Seu parceiro",
          body: n.repeatCount > 1 ? `${n.text} ×${n.repeatCount}` : n.text,
          duration: NOTIF_DISPLAY_MS,
        });
      } else {
        // Separate always-on-top window over other apps. Ensure it exists, then
        // emit the payload so NotificationOverlay renders it.
        await openNotifOverlay();
        await emitNotification({ ...n, dark });
      }
    }

    function armTimer() {
      if (timer != null) clearTimeout(timer);
      timer = window.setTimeout(() => {
        // Display window elapsed: drop the front card. The next live card
        // (onNotification fires again if one is queued) re-arms; an empty queue
        // is handled by the onQueueChange effect below, which closes the overlay.
        notificationQueue.shiftFront();
      }, NOTIF_DISPLAY_MS);
    }

    const offNotif = notificationQueue.onNotification((n) => {
      void surface(n, resolvedDark(settings.theme));
      // Optional sound (graceful no-op without the bundled asset — the toggle is
      // still useful, a `notif-chime.mp3` dropped into public/ activates it).
      if (settings.notif_sound_enabled) {
        try {
          const a = new Audio("/notif-chime.mp3");
          a.volume = 0.3;
          void a.play().catch(() => {});
        } catch {
          /* asset missing / autoplay blocked — silent */
        }
      }
      armTimer();
    });

    const offChange = notificationQueue.onQueueChange((q) => {
      if (q.length === 0) {
        if (timer != null) {
          clearTimeout(timer);
          timer = null;
        }
        // The overlay only exists to show a card; empty queue → tear it down.
        void closeNotifOverlay();
      }
    });

    // The user came back to Harbor while an overlay was up — close it; further
    // notifications will surface as in-app toasts (the surface choice is
    // re-evaluated per enqueue). Keeps a half-dismissed overlay from lingering.
    let offFocus: (() => void) | undefined;
    const win = getCurrentWindow();
    win
      .onFocusChanged(({ payload: focused }) => {
        if (focused) void closeNotifOverlay();
      })
      .then((un) => {
        offFocus = un;
      })
      .catch(() => {});

    // Honor an "Ignorar" click from the overlay window — drop the dismissed id
    // from the queue so it doesn't re-surface after its display window.
    let offDismiss: (() => void) | undefined;
    listen<{ id: string }>(NOTIF_DISMISS_EVENT, (e) => {
      const q = notificationQueue.getQueue();
      if (q[0]?.id === e.payload.id) notificationQueue.shiftFront();
      else notificationQueue.dismiss(e.payload.id);
    }).then((un) => {
      offDismiss = un;
    });

    return () => {
      offNotif();
      offChange();
      offFocus?.();
      offDismiss?.();
      if (timer != null) clearTimeout(timer);
    };
    // settings is read fresh inside the handlers via closure; re-subscribing on
    // every settings change is cheap and keeps the closures correct.
  }, [settings]);
}
