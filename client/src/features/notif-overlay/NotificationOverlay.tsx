/**
 * NotificationOverlay — the sole content of the always-on-top notification
 * window (`#/notif-overlay`). Mirrors the widget: it never opens a socket and
 * holds no store; the main window pushes each notification as a global
 * `harbor-notification` event (`services/notifOverlay.ts`).
 *
 * It renders one frosted card at a time: partner avatar + name + the message +
 * "Agora mesmo". Three actions:
 *  - "Responder" / "Abrir chat" → emit `harbor-navigate` (the main window listens,
 *    focuses itself, unminimizes, routes to #/chat) and dismiss the card.
 *  - "Ignorar" → emit `harbor-notif-dismiss` (the main window drops it from the
 *    queue slot rotation) and dismiss locally.
 *
 * Theme: the overlay applies `.dark` to its own <html> based on the payload's
 * `dark` flag (it can't read Settings — no store permission), same as the widget.
 * Until the first event arrives it renders nothing (the main window only created
 * the window because there was something to show, so a blank frame is momentary).
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

  // Apply the theme straight to <html> — there's no store in this realm.
  useEffect(() => {
    if (notif) document.documentElement.classList.toggle("dark", notif.dark);
    else
      document.documentElement.classList.toggle(
        "dark",
        systemPrefersDark(),
      );
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
    <div className="notif-enter window-main flex h-screen w-screen flex-col rounded-2xl p-0 text-[13px] leading-tight">
      <div className="notif-card flex items-start gap-3 rounded-2xl shadow-lg">
        {/* Drag handle / header strip so the card can be moved. */}
        <div
          data-tauri-drag-region
          className="flex items-center justify-between px-3 pt-2"
        >
          <span
            data-tauri-drag-region
            className="text-[10px] uppercase tracking-wide text-harbor-deep/70"
          >
            Harbor
          </span>
          <button
            onClick={dismiss}
            className="-mr-1 flex h-5 w-5 items-center justify-center rounded-full text-harbor-ink/50 hover:bg-harbor-sky/40 hover:text-harbor-ink"
            aria-label="Ignorar"
          >
            ✕
          </button>
        </div>

        {/* Body — partner avatar + name + message. */}
        <div className="flex items-start gap-3 px-3 pb-1">
          <div className="shrink-0">
            <Avatar src={notif.partnerAvatar} alt={notif.partnerName} size={48} />
          </div>
          <div className="min-w-0 flex-1">
            <p className="truncate font-semibold text-harbor-ink">
              {notif.partnerName?.trim() || "Seu parceiro"}
            </p>
            <p className="mt-0.5 break-words text-harbor-ink/80">
              {notif.text}
              <span className="ml-1 font-semibold text-harbor-deep">{repeat}</span>
            </p>
            <p className="mt-1 text-[10px] text-harbor-sec">Agora mesmo</p>
          </div>
        </div>

        {/* Actions — right-aligned, compact. */}
        <div className="flex items-center justify-end gap-2 px-3 pb-3">
          <button
            onClick={navigate}
            className="rounded-lg bg-harbor-deep px-3 py-1.5 text-xs font-medium text-white transition hover:bg-harbor-sea"
          >
            Responder
          </button>
          <button
            onClick={navigate}
            className="rounded-lg border border-harbor-sky px-3 py-1.5 text-xs font-medium text-harbor-deep transition hover:bg-harbor-surface-strong"
          >
            Abrir chat
          </button>
          <button
            onClick={dismiss}
            className="rounded-lg px-2 py-1.5 text-xs text-harbor-sec transition hover:text-harbor-ink"
          >
            Ignorar
          </button>
        </div>
      </div>
    </div>
  );
}
