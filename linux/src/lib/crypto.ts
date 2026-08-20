/**
 * End-to-end encryption wrapper around libsodium's anonymous sealed box.
 *
 * Harbor's relay only routes ciphertext. At pairing time both clients exchange
 * X25519 public keys (see lib/relay.ts / hooks/useChat.ts); message bodies are
 * then sealed client-side with `crypto_box_seal` and opened with
 * `crypto_box_seal_open`. Private keys never leave the client; the relay stores
 * only public keys, so it is fully key-blind.
 *
 * The project plan named "ChaCha20-Poly1305"; libsodium's sealed-box primitive
 * uses XSalsa20-Poly1305 AEAD instead. Both are authenticated encryption; the
 * deviation is intentional — `crypto_box_seal` is libsodium's native anonymous
 * sealed-box (ephemeral X25519 + AEAD per message) and is the standard choice
 * here. Keys are X25519 (Curve25519), exactly as the plan specifies.
 *
 * libsodium loads lazily (dynamic import + `sodium.ready`) so its WASM stays out
 * of the main bundle chunk and the first chat send only pays a one-time few-ms
 * init. The WASM is fetched from the tauri://localhost origin in the WebView.
 */
import type _Sodium from "libsodium-wrappers";

type Sodium = typeof _Sodium;

let sodium: Sodium | null = null;
let readyP: Promise<Sodium> | null = null;

/** Lazily load + init libsodium once; cached thereafter. Never call crypto at
 *  module load — always route through this so WASM is ready first. */
function ensureSodium(): Promise<Sodium> {
  if (sodium) return Promise.resolve(sodium);
  if (!readyP) {
    // Dynamic import keeps libsodium (+ its WASM) out of the main bundle chunk.
    readyP = import("libsodium-wrappers").then((mod) => {
      const s = (mod as { default?: Sodium } & Sodium).default ?? (mod as unknown as Sodium);
      return s.ready.then(() => {
        sodium = s;
        return sodium;
      });
    });
  }
  return readyP;
}

export interface Keypair {
  /** base64 X25519 public key — publish to the relay. */
  pub: string;
  /** base64 X25519 private key — NEVER send to the relay. */
  priv: string;
}

/** Generate a fresh X25519 keypair (base64-encoded). Call on first launch /
 *  after a wiped store, before publishing the pubkey at /register or /profile. */
export async function genKeypair(): Promise<Keypair> {
  const s = await ensureSodium();
  const kp = s.crypto_box_keypair();
  return { pub: s.to_base64(kp.publicKey), priv: s.to_base64(kp.privateKey) };
}

/** Seal a plaintext string to the partner's public key. Returns a base64
 *  sealed-box string (transport-only; the relay forwards it verbatim as `enc`). */
export async function sealTo(partnerPubB64: string, plaintext: string): Promise<string> {
  const s = await ensureSodium();
  const cipher = s.crypto_box_seal(s.from_string(plaintext), s.from_base64(partnerPubB64));
  return s.to_base64(cipher);
}

/** Open a sealed-box (base64) with our own keypair. Returns the plaintext, or
 *  null if decryption failed / the box was tampered with. Never throws — callers
 *  surface a placeholder bubble instead of dropping silently. */
export async function openFrom(
  myPrivB64: string,
  myPubB64: string,
  cipherB64: string,
): Promise<string | null> {
  try {
    const s = await ensureSodium();
    const plain = s.crypto_box_seal_open(
      s.from_base64(cipherB64),
      s.from_base64(myPubB64),
      s.from_base64(myPrivB64),
    );
    return s.to_string(plain);
  } catch {
    return null;
  }
}
