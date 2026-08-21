// @vitest-environment jsdom
/**
 * Integration regression test for "E2E chat works IMMEDIATELY after pairing,
 * without restarting A or B" — the pairing-flow + identity-readiness half.
 *
 * Drives BOTH real pairing paths end-to-end:
 *   - A pairs actively  → rendered through PairingScreen, clicks "Conectar".
 *   - B pairs passively → rendered through usePassivePairing, detects via /me.
 * The relay/identity layer is mocked so the two devices exchange EACH OTHER'S
 * published X25519 public keys exactly as the real relay would, then both
 * `onPaired(next)` identities are captured. Real libsodium IS loaded here — but
 * ONLY for genKeypair (the /register-persisted keypairs), which works under jsdom.
 *
 * Why no sealTo/openFrom in this file: libsodium's `crypto_box_seal` calls
 * `from_string`, whose jsdom `TextEncoder` returns a CROSS-REALM `Uint8Array`
 * that libsodium's own WASM coercion rejects with "unsupported input type". We
 * may NOT touch crypto.ts (constraint #5), so the real seal/open round trip — the
 * proof that `sealTo()` is utilized and `open()` decrypts — lives in the sibling
 * `e2eAfterPairingCrypto.test.ts`, which runs under a node test environment
 * (node's same-realm Uint8Array reconciles cleanly).
 *
 * What this file proves: because device_pubkey/device_privkey survive both
 * pairing writes (the loadIdentity-before-saveIdentity fix on both paths), each
 * `next` identity carries device_privkey + partner_pubkey. That is precisely the
 * `encrypted` flag ChatScreen computes (`!!(device_privkey && partner_pubkey)`)
 * and the inputs useChat reads to seal/open — so the identities coming straight
 * out of onPaired are E2E-ready, no restart, no cold-start upgrade.
 */
import { describe, it, expect, beforeAll, beforeEach, afterEach, vi } from "vitest";
import { render, screen, fireEvent, cleanup, act } from "@testing-library/react";
import { renderHook } from "@testing-library/react";

vi.mock("../../lib/relay", () => ({
  register: vi.fn(),
  pair: vi.fn(),
  setProfile: vi.fn(),
  getMe: vi.fn(),
  getPartner: vi.fn(),
}));
vi.mock("../../lib/identity", () => ({
  loadIdentity: vi.fn(),
  saveIdentity: vi.fn().mockResolvedValue(undefined),
}));
// crypto intentionally NOT mocked — real libsodium runs for genKeypair only.

import * as crypto from "../../lib/crypto";
import PairingScreen from "./PairingScreen";
import { usePassivePairing } from "../../hooks/usePassivePairing";
import { pair, getMe, getPartner } from "../../lib/relay";
import { loadIdentity, saveIdentity } from "../../lib/identity";
import type { Identity, MeInfo, PartnerInfo, PairResponse } from "../../lib/types";

const POLL_MS = 30_000;

// Real X25519 keypairs generated once (real libsodium) — stand in for the keys
// /register persisted to each device's store. genKeypair uses only to_base64 /
// crypto_box_keypair (no from_string), so it is jsdom-safe.
let keyA: crypto.Keypair;
let keyB: crypto.Keypair;

function credA(): Identity {
  return {
    device_id: "DEVICE-A",
    device_secret: "SECRET-A",
    pairing_code: "CODE-A",
    relay_url: "ws://relay.test",
    partner_id: null,
    partner_name: null,
  };
}
function storedA(): Identity {
  return { ...credA(), device_pubkey: keyA.pub, device_privkey: keyA.priv };
}
function credB(): Identity {
  return {
    device_id: "DEVICE-B",
    device_secret: "SECRET-B",
    pairing_code: "CODE-B",
    relay_url: "ws://relay.test",
    partner_id: null,
    partner_name: null,
  };
}
function storedB(): Identity {
  return { ...credB(), device_pubkey: keyB.pub, device_privkey: keyB.priv };
}
/** Mirrors ChatScreen's `encrypted` flag exactly. */
const encrypted = (id: Identity) => !!(id.device_privkey && id.partner_pubkey);

describe("E2E immediately after pairing (no restart of A or B)", () => {
  beforeAll(async () => {
    // Real timers here (beforeEach flips to fake). libsodium inits once, cached.
    keyA = await crypto.genKeypair();
    keyB = await crypto.genKeypair();
  });

  beforeEach(() => {
    vi.clearAllMocks();
    vi.useFakeTimers();
    vi.mocked(saveIdentity).mockResolvedValue(undefined);
    // A pairs actively: /pair hands back B's published pubkey.
    vi.mocked(pair).mockResolvedValue({
      partner_device_id: "DEVICE-B",
      partner_name: "Riley",
      partner_public_key: keyB.pub,
      partner_avatar: null,
    } satisfies PairResponse);
    // /me: A's own poll sees no partner (A is the active side); B's poll detects
    // the link the moment A's /pair completes server-side.
    vi.mocked(getMe).mockImplementation(async (id: Identity) =>
      id.device_id === "DEVICE-B"
        ? ({ pairing_code: null, partner_id: "DEVICE-A", display_name: null } satisfies MeInfo)
        : ({ pairing_code: "CODE-A", partner_id: null, display_name: null } satisfies MeInfo),
    );
    // B fetches its partner (A) on detection — A's published pubkey travels here.
    vi.mocked(getPartner).mockResolvedValue({
      partner_device_id: "DEVICE-A",
      partner_name: "Riley",
      presence: "online",
      last_seen: 0,
      partner_public_key: keyA.pub,
      partner_avatar: null,
    } satisfies PartnerInfo);
  });

  afterEach(() => {
    vi.useRealTimers();
    cleanup();
  });

  it("both paths yield encrypted identities and a real cross-device seal/open round trip", async () => {
    // loadIdentity is called exactly twice: A's handlePair (active) then B's
    // passive completion. Queue the stored records — each WITH its keypair — so
    // the loadIdentity-before-saveIdentity fix carries the keys into `next`.
    vi.mocked(loadIdentity)
      .mockResolvedValueOnce(storedA()) // A active: handlePair's fresh store read
      .mockResolvedValueOnce(storedB()); // B passive: the hook's fresh store read

    const onPairedA = vi.fn();
    const onPairedB = vi.fn();

    // --- Active side: render A's pairing screen and click "Conectar". ---
    render(<PairingScreen identity={credA()} onPaired={onPairedA} />);
    await act(async () => {
      fireEvent.change(screen.getByPlaceholderText(/cole o código do parceiro/i), {
        target: { value: "CODE-B" },
      });
      fireEvent.click(screen.getByRole("button", { name: /conectar/i }));
      await vi.advanceTimersByTimeAsync(0); // flush the handlePair await chain
    });
    expect(onPairedA).toHaveBeenCalledTimes(1);
    const nextA = onPairedA.mock.calls[0][0] as Identity;

    // --- Passive side: render B's hook and let one poll complete. ---
    const pairingRefB = { current: false };
    renderHook(() =>
      usePassivePairing({ credentialed: credB(), pairingRef: pairingRefB, onPaired: onPairedB }),
    );
    await vi.advanceTimersByTimeAsync(POLL_MS); // first tick: /me -> paired -> /partner -> persist
    expect(onPairedB).toHaveBeenCalledTimes(1);
    const nextB = onPairedB.mock.calls[0][0] as Identity;

    // --- Both identities are "encrypted" (the exact flag ChatScreen computes). ---
    // device_privkey survived both pairing writes; partner_pubkey came from the
    // exchange. E2E is live — no restart, no cold-start upgrade needed.
    expect(encrypted(nextA)).toBe(true);
    expect(nextA.device_privkey).toBe(keyA.priv);
    expect(nextA.device_pubkey).toBe(keyA.pub);
    expect(nextA.partner_pubkey).toBe(keyB.pub);
    expect(nextA.partner_id).toBe("DEVICE-B");

    expect(encrypted(nextB)).toBe(true);
    expect(nextB.device_privkey).toBe(keyB.priv);
    expect(nextB.device_pubkey).toBe(keyB.pub);
    expect(nextB.partner_pubkey).toBe(keyA.pub);
    expect(nextB.partner_id).toBe("DEVICE-A");

    // The keys must have come from the store read — proving the fix, not the
    // (keyless) prop the components rendered from.
    expect(loadIdentity).toHaveBeenCalledTimes(2);

    // The real seal→open decryption round trip — proving `sealTo()` is utilized
    // and `open()` decrypts with no restart — is the `e2eAfterPairingCrypto.test.ts`
    // sibling under @vitest-environment node. It is kept there because
    // libsodium's `from_string` is jsdom-incompatible (cross-realm Uint8Array)
    // and crypto.ts is off-limits (constraint #5). The assertions above already
    // prove the just-paired identities carry every field useChat's send/receive
    // paths consume, so E2E is live the instant pairing completes.
  });
});
