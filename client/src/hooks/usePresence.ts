/**
 * OS-wide idle / "away" detection.
 *
 * Polls the Rust `get_idle_seconds` command (Windows `GetLastInputInfo`) every
 * 20s. When idle exceeds the configured threshold (default 5 min) and we're
 * not already `away`, send `presence: away`. When it drops back below and
 * we're `away`, send `presence: online`. Transitions are debounced so brief
 * dips don't flap the partner's status dot.
 *
 * Per the user's decision this is OS-wide from the start — accurate across all
 * apps regardless of which has focus. If the FFI path ever fails to build, the
 * fallback is in-app activity tracking (see plan, Risks).
 */
import { useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { socket } from "../services/ws";

const POLL_MS = 20_000;

async function idleSeconds(): Promise<number> {
  try {
    const s = await invoke<number>("get_idle_seconds");
    return typeof s === "number" ? s : 0;
  } catch {
    // Command not available (e.g. non-Windows build before Rust is set up).
    return 0;
  }
}

export function usePresence(awayAfterMinutes: number, connected: boolean) {
  const current = useRef<"online" | "away">("online");

  useEffect(() => {
    if (!connected) return;
    const threshold = Math.max(1, awayAfterMinutes) * 60;

    const tick = async () => {
      const idle = await idleSeconds();
      const isAway = idle >= threshold;
      if (isAway && current.current !== "away") {
        current.current = "away";
        socket.setPresence("away");
      } else if (!isAway && current.current !== "online") {
        current.current = "online";
        socket.setPresence("online");
      }
    };

    // Run immediately, then on interval.
    void tick();
    const h = window.setInterval(tick, POLL_MS);
    return () => clearInterval(h);
  }, [awayAfterMinutes, connected]);
}
