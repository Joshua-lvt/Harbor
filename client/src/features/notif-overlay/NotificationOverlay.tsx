/**
 * NotificationOverlay — the sole content of the always-on-top notification
 * window (`#/notif-overlay`). Mirrors the widget: it never opens a socket and
 * holds no store; the main window pushes each notification as a global
 * `harbor-notification` event (`services/notifOverlay.ts`).
 *
 * Harbor quick-messages are NOT small corner toasts and NOT OS notifications —
 * the whole point is that the message LANDS on the partner's screen, big and
 * unmissable. So this renders one large centered card with the message set in a
 * huge font, over the partner's avatar + name, with a gentle scale/fade-in.
 *
 * Three actions:
 *  - "Responder" → emit `harbor-navigate` (the main window listens, focuses
 *    itself, unminimizes, routes to #/chat) and dismiss the card.
 *  - "Ignorar" → emit `harbor-notif-dismiss` (the main window drops it from the
 *    queue) and dismiss locally.
 *
 * Theme: the overlay applies `.dark` to its own <html> based on the payload's
 * `dark` flag (it can't read Settings — no store permission), same as the widget.
 */
import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { Avatar } from "../../components/Avatar";
import { systemPrefersDark } from "../../lib/theme";
import {
  emit,
  type NotifPayload,
  NOTIF_NAVIGATE_EVENT,
  NOTIF_DISMISS_EVENT,
} from "../../services/notifOverlay";
import { applyCustomization, DEFAULT_CUSTOMIZATION } from "../../lib/customization";

interface OverlayNotif extends NotifPayload {
  /** Whether to render in dark mode (resolved by the main window; the overlay
   *  has no Settings store). */
  dark: boolean;
}

export default function NotificationOverlay() {
  const [notif, setNotif] = useState<OverlayNotif | null>(null);
  // Track the active id so a late duplicate event doesn't clobber a card the
  // user already dismissed locally (prevents a dismissed toast re-animating).
  const [activeId, setActiveId] = useState<string | null>(null);

  useEffect(() => {
    let off: (() => void) | undefined;
    listen<OverlayNotif>("harbor-notification", (e) => {
      const p = e.payload;
      setNotif(p);
      setActiveId(p.id);
    }).then((un) => {
      off = un;
    });
    return () => off?.();
  }, []);

  // Apply the theme straight to <html> — there's no store in this realm. The
  // customization (Personalização) rides the same payload so quick-messages land
  // in the partner's custom palette too; like `dark`, it's resolved by the main
  // window. Falls back to ocean until the first notification carries a palette.
  useEffect(() => {
    const dark = notif ? notif.dark : systemPrefersDark();
    document.documentElement.classList.toggle("dark", dark);
    applyCustomization(notif?.customization ?? DEFAULT_CUSTOMIZATION, dark);
  }, [notif]);

  function navigate() {
    if (!notif) return;
    void emit(NOTIF_NAVIGATE_EVENT, { id: notif.id });
    dismiss();
  }

  function dismiss() {
    if (!notif) return;
    const id = notif.id;
    setActiveId(null);
    setNotif(null);
    void emit(NOTIF_DISMISS_EVENT, { id });
  }

  if (!notif || notif.id !== activeId) {
    // Nothing to show — render a transparent frame (the parent window is
    // frameless + transparent, so this is invisible). The main window will
    // close the overlay once the queue is fully drained.
    return <div className="h-screen w-screen" />;
  }

  const repeat = notif.repeatCount > 1 ? ` ×${notif.repeatCount}` : "";

  return (
    <div className="notif-enter window-main flex h-screen w-screen items-center justify-center p-6">
      <div className="notif-card notif-giant flex w-full max-w-3xl flex-col items-center gap-6 rounded-3xl px-10 py-12 text-center shadow-2xl">
        {/* Header — Harbor wordmark + close, draggable. */}
        <div
          data-tauri-drag-region
          className="flex w-full items-center justify-between"
        >
          <span
            data-tauri-drag-region
            className="text-sm uppercase tracking-[0.2em] text-harbor-ice"
          >
            Harbor
          </span>
          <button
            onClick={dismiss}
            className="flex h-9 w-9 items-center justify-center rounded-full text-harbor-ink/60 transition hover:bg-harbor-sky/30 hover:text-harbor-ink"
            aria-label="Ignorar"
          >
            ✕
          </button>
        </div>

        {/* Partner identity — avatar + name */}
        <div className="flex flex-col items-center gap-3">
          <Avatar src={notif.partnerAvatar} alt={notif.partnerName} size={88} />
          <p className="text-xl font-bold text-harbor-ink">
            {notif.partnerName?.trim() || "Seu parceiro"}
          </p>
          <p className="text-xs text-harbor-sec">Agora mesmo</p>
        </div>

        {/* The message — GIGANTE. This is the whole point of the feature. */}
        <div className="min-w-0 flex-1 py-2">
          <p className="notif-message break-words text-5xl font-extrabold leading-tight text-harbor-deep">
            {notif.text}
            <span className="ml-2 align-middle text-3xl text-harbor-sea">
              {repeat}
            </span>
          </p>
        </div>

        {/* Actions */}
        <div className="flex items-center justify-center gap-3">
          <button
            onClick={navigate}
            className="rounded-xl bg-harbor-deep px-6 py-3 text-base font-semibold text-white transition hover:bg-harbor-sea"
          >
            Responder
          </button>
          <button
            onClick={dismiss}
            className="rounded-xl border border-harbor-sky px-6 py-3 text-base font-medium text-harbor-deep transition hover:bg-harbor-surface-strong"
          >
            Ignorar
          </button>
        </div>
      </div>
    </div>
  );
}
