/**
 * Autostart on Windows boot via @tauri-apps/plugin-autostart.
 *
 * The Rust side registers the plugin in `setup` (desktop-only). Here we just
 * expose enable/disable/isEnabled for the Settings UI. `enable()` is idempotent
 * — safe to call on every boot if the toggle is on.
 */
import { enable, disable, isEnabled } from "@tauri-apps/plugin-autostart";

export async function setAutostart(on: boolean): Promise<void> {
  if (on) {
    await enable();
  } else {
    await disable();
  }
}

export async function isAutostartEnabled(): Promise<boolean> {
  try {
    return await isEnabled();
  } catch {
    // Plugin not registered or not on a desktop target.
    return false;
  }
}
