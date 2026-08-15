/**
 * WidgetScreen — the sole content of the always-on-top widget window
 * (`#/widget`). It never opens a socket and never reads the Tauri store;
 * everything it shows arrives as a `harbor-widget-state` event from the main
 * window (see services/widget.ts + App.tsx reconciler).
 *
 * Renders a tiny borderless, draggable (header `data-tauri-drag-region`) card:
 * partner avatar → name + presence dot → current app label, a ✕ that emits
 * `harbor-widget-closed` (the main window listens, flips `widget_enabled` to
 * false, and persists — Settings stays the single source of truth), and a
 * bottom-right resize grip that starts a native `startResizeDragging('SouthEast')`
 * — the window is frameless + transparent (no OS resize border to grab), so the
 * grip is the only way to resize it. Theme is applied directly here: the widget
 * has no Settings store, so the snapshot carries a `dark` flag and this screen
 * toggles `.dark` on its own documentElement.
 *
 * `window-main` paints the ocean gradient; in dark mode the tokens flip via the
 * `.dark` class, same as the main window. Until the first state event arrives
 * we show a quiet "Carregando…" placeholder so the window is never blank.
 */
import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Avatar } from "../../components/Avatar";
import { friendlyName, detectGame } from "../../lib/appNames";
import { GeneratedAppIcon } from "../../lib/appIconCache";
import { systemPrefersDark } from "../../lib/theme";
import { emitWidgetClosed, type WidgetSnapshot } from "../../services/widget";

// Start `dark` from the OS preference so the very first paint matches what
// main.tsx already applied to <html> — no flash before the first snapshot
// arrives. The first `harbor-widget-state` event then corrects it to whatever
// the main window's resolved theme actually is.
const INITIAL: WidgetSnapshot = {
  connected: false,
  partnerPresence: "offline",
  partnerActivity: null,
  partnerActivityIcon: null,
  partnerName: "",
  partnerAvatar: null,
  myName: "",
  myAvatar: null,
  myActivity: null,
  voiceStatus: "reconnecting",
  dark: systemPrefersDark(),
};

/** Kick a native South-East resize drag on the widget window. The window is
 *  frameless + transparent (no OS resize border to grab), so the only way to
 *  resize it is this corner grip. `startResizeDragging` hands the drag to the OS,
 *  which tracks the mouse and resizes the window edge-by-edge — just like
 *  dragging a normal window's bottom-right border. Best-effort (non-Tauri → no-op). */
function startWidgetResize(e: React.PointerEvent): void {
  // A pointerdown (not click) so the drag begins immediately where the user
  // grabs, and so a stray click without movement doesn't fire a no-op resize.
  if (e.button !== 0) return;
  e.preventDefault();
  void getCurrentWindow().startResizeDragging("SouthEast").catch(() => {});
}

export default function WidgetScreen() {
  const [snap, setSnap] = useState<WidgetSnapshot>(INITIAL);

  useEffect(() => {
    // Apply the theme straight to <html> as snapshots arrive — there's no store
    // on the widget side, so this is the only place that flips .dark here.
    document.documentElement.classList.toggle("dark", snap.dark);
  }, [snap.dark]);

  useEffect(() => {
    let off: (() => void) | undefined;
    listen<WidgetSnapshot>("harbor-widget-state", (e) => setSnap(e.payload)).then((un) => {
      off = un;
    });
    return () => off?.();
  }, []);

  const presence =
    snap.partnerPresence === "online"
      ? "Online"
      : snap.partnerPresence === "away"
        ? "Ausente"
        : "Offline";
  const dot =
    snap.partnerPresence === "online"
      ? "dot-online"
      : snap.partnerPresence === "away"
        ? "dot-away"
        : "dot-offline";

  const partnerName = snap.partnerName?.trim() || "Seu parceiro";
  const game = detectGame(snap.partnerActivity);
  const activityLabel = snap.connected
    ? game
      ? `🎮 ${game}`
      : snap.partnerActivity
        ? friendlyName(snap.partnerActivity)
        : "Por aqui"
    : "Offline";

  // Feature 4: a 20px app icon before the activity label. The main window
  // resolved it from the icon cache and pushed it in the snapshot (cache-only —
  // the widget has no socket/store to resolve itself). The generated fallback
  // (colored circle + initial) shows when no icon is cached for this exe.
  const activityIcon =
    snap.connected && snap.partnerActivity
      ? snap.partnerActivityIcon
        ? (
            <img
              src={snap.partnerActivityIcon}
              alt=""
              className="inline-block h-5 w-5 shrink-0 rounded object-contain align-middle"
              draggable={false}
            />
          )
        : (
            <GeneratedAppIcon
              exe={snap.partnerActivity}
              size={20}
              className="inline-block align-middle"
            />
          )
      : null;

  return (
    <div className="window-main relative flex h-screen w-screen flex-col rounded-2xl p-0 text-[13px] leading-tight">
      {/* Drag handle / header — the whole top strip is draggable. The ✕ sits
          outside the drag region so the click doesn't start a drag. */}
      <div
        data-tauri-drag-region
        className="flex items-center justify-between px-2 pt-2"
      >
        <span
          data-tauri-drag-region
          className="text-[10px] uppercase tracking-wide text-harbor-deep/60"
        >
          Harbor
        </span>
        <button
          onClick={() => void emitWidgetClosed()}
          className="-mr-1 flex h-5 w-5 items-center justify-center rounded-full text-harbor-ink/50 hover:bg-harbor-sky/40 hover:text-harbor-ink"
          aria-label="Fechar widget"
        >
          ✕
        </button>
      </div>

      {/* Body — partner orb + names + current app. */}
      <div className="flex items-center gap-3 px-3 pb-3">
        <div className="relative shrink-0">
          <Avatar src={snap.partnerAvatar} alt={partnerName} size={48} />
          <span
            className={`absolute bottom-0 right-0 h-3 w-3 rounded-full ${dot} ring-2 ring-harbor-surface`}
          />
        </div>
        <div className="min-w-0 flex-1">
          <p className="truncate font-semibold text-harbor-ink">{partnerName}</p>
          <p className="truncate text-xs text-harbor-sec">
            <span className="inline-block align-middle mr-1">{presence}</span>
            <span className="inline-block align-middle">· {activityIcon}{activityLabel}</span>
          </p>
        </div>
      </div>

      {/* Resize grip — bottom-right corner. The window is frameless + transparent
          so there's no OS border to grab; this ↘ handle starts a native South-East
          resize drag (`startResizeDragging`). It doubles as a visual affordance,
          and sits OUTSIDE the drag region so dragging-to-move vs resize don't
          clash. Only shows on hover so the tiny widget stays clean at rest. */}
      <div
        onPointerDown={startWidgetResize}
        className="absolute bottom-0 right-0 flex h-4 w-4 cursor-nwse-resize items-end justify-end p-0.5 opacity-0 transition-opacity hover:opacity-60"
        aria-label="Redimensionar widget"
      >
        <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden>
          <path d="M9 1 L1 9 M9 5 L5 9" stroke="currentColor" strokeWidth="1.2" strokeLinecap="round" className="text-harbor-deep/70" />
        </svg>
      </div>
    </div>
  );
}
