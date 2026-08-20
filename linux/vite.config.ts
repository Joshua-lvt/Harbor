import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

// Tauri dev host (for mobile / remote dev). Empty in normal desktop dev.
const host = process.env.TAURI_DEV_HOST;

// https://vitejs.dev/config/
export default defineConfig({
  plugins: [react(), tailwindcss()],

  // Tauri expects a fixed port and a clear screen for its dev flow.
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    // Bind IPv4 loopback explicitly (desktop dev). The default `false` lets the
    // Vite server land on IPv6 `::1` only; on Windows the WebView2 in `tauri
    // dev` then resolves `localhost` → `127.0.0.1` (IPv4) and fails to connect,
    // so the main window opens to a BLANK screen ("tela sem nada") while an
    // orphan `vite` still owns :1420. Pinning host + devUrl to 127.0.0.1 makes
    // the dev loop deterministic regardless of Windows' IPv4/IPv6 ordering.
    // TAURI_DEV_HOST keeps precedence for the mobile/remote dev path.
    host: host || "127.0.0.1",
    hmr: host
      ? { protocol: "ws", host, port: 1421 }
      : undefined,
    watch: {
      // Don't watch the Rust side.
      ignored: ["**/src-tauri/**"],
    },
  },
  envPrefix: ["VITE_", "TAURI_ENV_*"],
});
