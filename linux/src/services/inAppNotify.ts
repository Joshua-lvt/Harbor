/**
 * inAppNotify — bridge from the app-lifetime singletons (messages.ts, notify.ts)
 * to the in-app Harbor Toaster (Bug 4).
 *
 * The singletons are plain modules with no React context, so they can't call
 * `useToast()` directly. Instead they emit a global Tauri event; App.tsx (which
 * owns the ToastProvider) listens and pushes a Harbor-styled toast. This is the
 * "notificações padrão do Harbor" surface the user asked for — when the app is
 * focused we show this instead of the OS toast.
 */
import { emit } from "@tauri-apps/api/event";

export const INAPP_NOTIFICATION_EVENT = "harbor-inapp-notification";

export interface InAppNotification {
  title: string;
  body?: string;
  icon?: string;
}

/** Emit an in-app Harbor toast. Best-effort; non-Tauri builds no-op. */
export function inAppNotify(n: InAppNotification): void {
  void emit(INAPP_NOTIFICATION_EVENT, n).catch(() => {});
}
