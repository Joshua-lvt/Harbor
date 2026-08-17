/**
 * Small, dependency-free helpers shared by the Harbor Worker + Durable Objects.
 *
 * Ported from `server/app/security.py` + `pairing.py` so the Cloudflare backend issues
 * the same `HARBOR-XXXX-XXXX` codes and 256-bit URL-safe secrets the FastAPI relay did,
 * and authenticates with the same constant-time compare (`server/app/security.py:13`).
 */

/** Unambiguous alphabet: excludes 0, O, 1, I so codes read/relay aloud cleanly. */
const SAFE_ALPHABET = "ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
const CODE_PREFIX = "HARBOR";

/** Largest inbound WS frame we accept (256 KB). Oversized frames are rejected without
 *  dropping the socket — matches FastAPI's "one bad message must not kill the socket"
 *  posture (`server/app/ws.py:272`). Configurable via `HARBOR_MAX_FRAME_BYTES`. */
export const DEFAULT_MAX_FRAME_BYTES = 262_144;

/** A 32-char group drawn from the safe alphabet, via the runtime CSPRNG. */
function randGroup(n = 4): string {
  const out: string[] = [];
  const max = Math.floor(0xff / SAFE_ALPHABET.length) * SAFE_ALPHABET.length;
  const buf = new Uint8Array(n);
  // Fill extra so we can rejection-sample within the unbiased range below.
  const fill = new Uint8Array(n * 4);
  crypto.getRandomValues(fill);
  let taken = 0;
  for (let i = 0; i < fill.length && taken < n; i++) {
    const v = fill[i];
    if (v < max) out[taken++] = SAFE_ALPHABET[v % SAFE_ALPHABET.length];
  }
  // Should never need more than n*4 samples; fall back to modulo bias-free-ish retry.
  while (taken < n) {
    const b = new Uint8Array(1);
    crypto.getRandomValues(b);
    if (b[0] < max) out[taken++] = SAFE_ALPHABET[b[0] % SAFE_ALPHABET.length];
  }
  return out.join("");
}

/** Generate a fresh `HARBOR-XXXX-XXXX` pairing code (`pairing.py:35`). */
export function generateCode(): string {
  return `${CODE_PREFIX}-${randGroup()}-${randGroup()}`;
}

/** URL-safe base64 (no padding) of 32 random bytes — the 256-bit device secret (`security.py:9`). */
export function newSecret(): string {
  const bytes = new Uint8Array(32);
  crypto.getRandomValues(bytes);
  let bin = "";
  for (let i = 0; i < bytes.length; i++) bin += String.fromCharCode(bytes[i]);
  return btoa(bin).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

/** Constant-time string comparison (faithful to `hmac.compare_digest`). Returns false on
 *  null/empty or length mismatch — the length check leaks length-equality only, which is
 *  fine here since secrets are a fixed ~43 chars. */
export function verifySecret(stored: string | null | undefined, presented: string | null | undefined): boolean {
  if (!stored || !presented) return false;
  if (stored.length !== presented.length) return false;
  let mismatch = 0;
  for (let i = 0; i < stored.length; i++) mismatch |= stored.charCodeAt(i) ^ presented.charCodeAt(i);
  return mismatch === 0;
}

/** Deterministic pair routing key: the two device ids, sorted, joined by ":". Same input
 *  from either side yields the same key so both devices hash to one HarborPair instance. */
export function pairKey(a: string, b: string): string {
  return [a, b].sort().join(":");
}

/** Wall-clock seconds with fractional, matching Python's `time.time()`. Workers' `Date.now()`
 *  is frozen for a single handler invocation and advances across awaits; fine for ts. */
export function nowTs(): number {
  return Date.now() / 1000;
}

/** Parse + validate a single inbound WS frame. Returns the typed message or `null` if the
 *  frame is not valid JSON. The DO decides what to do with a `null` (skip silently) vs a
 *  validation error (reply `{type:"error"}`). Mirrors `ws.py:264-270`. */
export function parseFrame(message: ArrayBuffer | string): unknown | null {
  let text: string;
  if (typeof message === "string") text = message;
  else text = new TextDecoder().decode(message);
  try {
    return JSON.parse(text);
  } catch {
    return null;
  }
}
