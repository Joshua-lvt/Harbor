import React from "react";
import { createRoot } from "react-dom/client";

// Uncaught errors append a fixed overlay (never replace #root — that unmounts
// React and leaves a blank ocean gradient after UpdateGate passes).
(function installBootErrorOverlay() {
  function show(label: string, detail: unknown) {
    const stack = detail instanceof Error ? `${detail.message}\n${detail.stack ?? ""}` : String(detail);
    console.error(`[Harbor boot] ${label}`, detail);
    let layer = document.getElementById("harbor-boot-error");
    if (!layer) {
      layer = document.createElement("pre");
      layer.id = "harbor-boot-error";
      layer.style.cssText =
        "position:fixed;inset:0;margin:0;padding:16px;overflow:auto;z-index:2147483647;background:#fff;color:#7a1a1a;font:12px/1.4 ui-monospace,Consolas,monospace;white-space:pre-wrap;pointer-events:auto;";
      document.body.appendChild(layer);
    }
    layer.textContent = `[Harbor boot] ${label}\n\n${stack}`;
  }
  window.addEventListener("error", (e) => show("window.error", e.error ?? e.message));
  window.addEventListener("unhandledrejection", (e) => show("unhandledrejection", e.reason));
})();
import App from "./App";
import WidgetScreen from "./features/widget/WidgetScreen";
import NotificationOverlay from "./features/notif-overlay/NotificationOverlay";
import { ToastProvider } from "./components/Toaster";
import { loadSettings } from "./lib/identity";
import { applyTheme, systemPrefersDark } from "./lib/theme";
import UpdateGate from "./features/update/UpdateGate";
import "./style.css";

// The widget + the notification overlay are SECOND Tauri windows loading the
// same bundle at `#/widget` / `#/notif-overlay`. They must NOT mount App — App's
// boot effect opens a socket with this device's id, and the relay only allows
// one live socket per device (a second would evict the main window's). So each
// gets its own minimal root that never touches the socket, store, tray, or
// notifications — the overlay listens for `harbor-notification` events pushed by
// the main window (see services/notifOverlay.ts), exactly like the widget.
//
// The MAIN window wraps <App/> in <UpdateGate> (Fase 2): the gate runs a signed
// check() before letting App mount, so in any non-up-to-date state there is NO
// socket, no loadIdentity, no relay traffic at all. The widget/overlay are not
// gated — they own no socket/store and so can't bypass the block from a side
// window (see UpdateGate.tsx for the exact state table).
const hash = typeof window !== "undefined" ? window.location.hash : "";
const isWidget = hash.startsWith("#/widget");
const isNotifOverlay = hash.startsWith("#/notif-overlay");

// Paint an initial theme before React mounts to avoid a light flash on a dark
// OS. The main window refines to the saved setting once the store loads; the
// widget has no store permission, so it starts from the OS preference and the
// first `harbor-widget-state` event corrects it (WidgetScreen applies snap.dark).
applyTheme(systemPrefersDark() ? "dark" : "light");
if (!isWidget && !isNotifOverlay) {
  void loadSettings()
    .then((s) => applyTheme(s.theme))
    .catch(() => {});
}

createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    {isWidget ? (
      <WidgetScreen />
    ) : isNotifOverlay ? (
      <NotificationOverlay />
    ) : (
      <ToastProvider>
        <UpdateGate>
          <App />
        </UpdateGate>
      </ToastProvider>
    )}
  </React.StrictMode>,
);
