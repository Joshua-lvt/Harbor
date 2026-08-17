/**
 * useHarborNotifications — mounted ONCE in App for the app lifetime. Drives
 * the display of Harbor quick-message notifications (Feature 5).
 *
 * Unlike a normal chat message, a Harbor quick-message is NEVER an in-app toast
 * and NEVER an OS notification. It is the giant, centered, always-on-top
 * `notif-overlay` Tauri window — the message lands on the partner's screen big
 * and unmissable, whether Harbor is focused or not. So the focus/visibility
 * branch (in-app Toaster vs overlay) is gone: every enqueued quick-message
 * goes to the overlay, full stop.
 *
 * Quick-messages are E2E-encrypted and notification-only (detected in the
 * `messages` singleton's `harbor_notify` inner kind → `notificationQueue.enqueue`).
 * This hook drives the display-slot rotation: each enqueued card shows for
 * NOTIF_DISPLAY_MS, then the front is shifted so the next card surfaces.
 * Collapses (identical preset within 5s) update the live card in place and
 * reset the timer. When the queue drains the overlay window is closed.
 *
 * The overlay is a separate Tauri window = a separate JS realm, so the queue
 * singleton can't be shared; each new notification is broadcast to the overlay
 * via `emitNotification` (a global Tauri event), and dismissed-via-overlay is
 * honored through the `harbor-notif-dismiss` event the overlay emits.
 */
import { useEffect } from "react";
import { listen } from "@tauri-apps/api/event";
import { notificationQueue, NOTIF_DISPLAY_MS } from "../services/notificationQueue";
import {
  openNotifOverlay,
  closeNotifOverlay,
  emitNotification,
  type NotifPayload,
  NOTIF_DISMISS_EVENT,
} from "../services/notifOverlay";
import { customizationOf } from "../lib/customization";
import type { Settings } from "../lib/types";

function resolvedDark(theme: Settings["theme"]): boolean {
  return (
    theme === "dark" ||
    (theme === "system" &&
      window.matchMedia("(prefers-color-scheme: dark)").matches)
  );
}

export function useHarborNotifications(settings: Settings): void {
  useEffect(() => {
    let timer: number | null = null;

    /** Surface one notification on the giant overlay window. Both `dark` and
     *  `customization` are resolved here (the overlay has no Settings store) so
     *  the card lands in the user's current palette + theme, not ocean. */
    async function surface(n: NotifPayload, dark: boolean) {
      await openNotifOverlay();
      await emitNotification({ ...n, dark, customization: customizationOf(settings) });
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

    // The user came back to Harbor while an overlay was up — leave the overlay
    // open. A quick-message is meant to be seen; focusing the app does NOT
    // dismiss it. (The old behavior closed the overlay on focus so the in-app
    // toaster could take over, but the toaster path is gone now.)

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
      offDismiss?.();
      if (timer != null) clearTimeout(timer);
    };
    // settings is read fresh inside the handlers via closure; re-subscribing on
    // every settings change is cheap and keeps the closures correct.
  }, [settings]);
}
