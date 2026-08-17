/**
 * Vitest config for harbor-cloud.
 *
 * The `cloudflareTest` Vite plugin (pool @cloudflare/vitest-pool-workers, vitest
 * 4.x) runs each test inside a Miniflare isolate that materializes the Durable
 * Object bindings + vars from `wrangler.jsonc` — so tests exercise the real
 * Worker router + DOs (registry.ts / pair.ts) in-memory, no deploy needed.
 *
 * Storage isolation is per test *file* in the new pool (matches vitest's model).
 * Within a file we reset DO storage between cases with `reset()` from
 * `cloudflare:test` in `afterEach` — see test/registry.test.ts + pair.test.ts.
 */
import { cloudflareTest } from "@cloudflare/vitest-pool-workers";
import { defineConfig } from "vitest/config";

export default defineConfig({
  plugins: [
    cloudflareTest({
      // `wrangler.jsonc` drives bindings/vars/exports so tests match prod. Its
      // `main` (src/index.ts) is auto-discovered, giving us `SELF` + the DO
      // bindings under `cloudflare:test`'s `env`.
      wrangler: { configPath: "./wrangler.jsonc" },
    }),
  ],
});
