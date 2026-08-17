/**
 * Tray icon presence — mirrors the partner's presence into the system tray
 * icon + tooltip via the Rust `set_tray_presence` command.
 *
 * Unlike notifications (gated per-category by Settings toggles), the tray is
 * always visible, so it should always reflect the partner's current state.
 * Subscribes to the same in-process `socket.onEvent` stream that
 * `attachNotifications` uses; both run in parallel.
 */
import { invoke } from "@tauri-apps/api/core";
import type { ServerEvent } from "../lib/types";

export function attachTrayPresence(
  subscribe: (h: (e: ServerEvent) => void) => () => void,
): () => void {
  return subscribe((e) => {
    if (e.type === "presence") {
      void invoke("set_tray_presence", { state: e.state });
    } else if (e.type === "last_seen") {
      // Cold-start reply carries the partner's current presence — seed the tray
      // even if no transition has fired yet (partner already online at launch).
      void invoke("set_tray_presence", { state: e.presence });
    }
  });
}
