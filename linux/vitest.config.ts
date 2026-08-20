import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";

// Test config is SEPARATE from vite.config.ts (the app build). `npm run build`
// (tsc && vite build) reads vite.config.ts only and stays unaffected; vitest
// reads this file. The client never had a test runner before passive-pairing
// detection required one — this is the minimal vitest + jsdom setup for the
// unit tests under src/. `globals: false` keeps imports explicit (no env types
// needed in tsconfig.json, so the build's tsc doesn't see vitest types either).
export default defineConfig({
  plugins: [react()],
  test: {
    environment: "jsdom",
    globals: false,
    include: ["src/**/*.test.{ts,tsx}"],
  },
});
