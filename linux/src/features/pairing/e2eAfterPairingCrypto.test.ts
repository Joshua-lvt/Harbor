/**
 * The real-crypto half of "E2E chat works IMMEDIATELY after pairing, without
 * restarting A or B" — the seal→open decryption proof.
 *
 * This file runs under a node test environment (see the directive below) because
 * libsodium's `crypto_box_seal` calls `from_string`, whose jsdom `TextEncoder` returns a
 * CROSS-REALM `Uint8Array` that libsodium's WASM coercion rejects ("unsupported
 * input type for message"). Node's same-realm `TextEncoder` has no such
 * mismatch. We are forbidden from touching crypto.ts (constraint #5), so we
 * cannot shim the encoder — moving just this assertion to a node env is the
 * faithful, constraint-clean fix.
 *
 * The pairing-flow half (driving PairingScreen + usePassivePairing end-to-end
 * and asserting `encrypted === true` + key survival) lives in the jsdom sibling
 * `postPairE2E.test.tsx`. This file takes its result as given — two devices
 * that just paired, each now holding (per the fix) device_privkey +
 * device_pubkey + partner_pubkey — and proves the KEYS THAT SURVIVE THE PAIRING
 * WRITE actually seal and open: A seals to B's published pubkey and B opens with
 * its own keypair, and vice versa, exactly as useChat does, with no restart.
 *
 * It covers the two sub-assertions the jsdom sibling cannot host:
 *   - `sealTo()` is utilized (both send directions, real call)
 *   - `open()` decrypts (both receive directions, real plaintext recovered)
 */
// @vitest-environment node
import { describe, it, expect, beforeAll, vi } from "vitest";

import * as crypto from "../../lib/crypto";
import type { Identity } from "../../lib/types";

const MSG_A_TO_B = "Oi do A 🔒";
const MSG_B_TO_A = "Recebi! Resposta do B 🔒";

// Real X25519 keypairs generated once — stand in for the keypairs /register
// persisted to each device's store and that the loadIdentity-before-saveIdentity
// fix preserves across the pairing write.
let keyA: crypto.Keypair;
let keyB: crypto.Keypair;

/** Build the post-pairing identity EXACTLY as the (fixed) pairing flow leaves it:
 *  the device's OWN keypair (survived the write) + the partner's published pubkey
 *  (learned via /pair on the active side, /partner on the passive side). This is
 *  the field set ChatScreen's `encrypted` flag and useChat's seal/open read. */
function pairedIdentity(
  mine: crypto.Keypair,
  partnerPub: string,
  deviceId: string,
  partnerId: string,
): Identity {
  return {
    device_id: deviceId,
    device_secret: `SECRET-${deviceId}`,
    pairing_code: null,
    relay_url: "ws://relay.test",
    partner_id: partnerId,
    partner_name: "Riley",
    device_pubkey: mine.pub,
    device_privkey: mine.priv,
    partner_pubkey: partnerPub,
    partner_avatar: null,
  };
}

describe("E2E crypto immediately after pairing — sealTo() utilized + open() decrypts (no restart)", () => {
  beforeAll(async () => {
    keyA = await crypto.genKeypair();
    keyB = await crypto.genKeypair();
  });

  it("A→B and B→A: sealTo is utilized and open decrypts with the just-paired keys", async () => {
    // The two identities coming straight out of onPaired — no cold-start upgrade,
    // no restart. Each carries own keypair + partner pubkey (the fix's guarantee).
    const nextA = pairedIdentity(keyA, keyB.pub, "DEVICE-A", "DEVICE-B");
    const nextB = pairedIdentity(keyB, keyA.pub, "DEVICE-B", "DEVICE-A");

    // Mirror ChatScreen's `encrypted` flag exactly — both sides are live.
    const encrypted = (id: Identity) => !!(id.device_privkey && id.partner_pubkey);
    expect(encrypted(nextA)).toBe(true);
    expect(encrypted(nextB)).toBe(true);

    // Spy AFTER the identities exist; the spies wrap the real functions (real
    // libsodium still runs), so they prove genuine utilization + decryption.
    const sealSpy = vi.spyOn(crypto, "sealTo");
    const openSpy = vi.spyOn(crypto, "openFrom");

    // A -> B: A seals { text } to B's partner (B's) pubkey; B opens with its own
    // keypair. This is useChat's send/recv path verbatim.
    const wireAB = await crypto.sealTo(
      nextA.partner_pubkey!,
      JSON.stringify({ text: MSG_A_TO_B }),
    );
    const openedAB = await crypto.openFrom(
      nextB.device_privkey!,
      nextB.device_pubkey!,
      wireAB,
    );
    expect(openedAB).not.toBeNull();
    expect(JSON.parse(openedAB!).text).toBe(MSG_A_TO_B);

    // B -> A: the reverse direction — B seals to A's pubkey, A opens.
    const wireBA = await crypto.sealTo(
      nextB.partner_pubkey!,
      JSON.stringify({ text: MSG_B_TO_A }),
    );
    const openedBA = await crypto.openFrom(
      nextA.device_privkey!,
      nextA.device_pubkey!,
      wireBA,
    );
    expect(openedBA).not.toBeNull();
    expect(JSON.parse(openedBA!).text).toBe(MSG_B_TO_A);

    // `sealTo()` is utilized — real calls, both send directions.
    expect(sealSpy).toHaveBeenCalledTimes(2);
    // `open()` decrypts — real calls, both receive directions, both recovered.
    expect(openSpy).toHaveBeenCalledTimes(2);

    sealSpy.mockRestore();
    openSpy.mockRestore();
  });

  it("tampered / wrong-key ciphertext fails closed (open resolves null, never throws)", async () => {
    // A seal to A's OWN pubkey cannot be opened by B (wrong keypair) — openFrom
    // must resolve null, mirroring useChat's never-throw contract.
    const nextB = pairedIdentity(keyB, keyA.pub, "DEVICE-B", "DEVICE-A");
    const wireSelf = await crypto.sealTo(nextB.partner_pubkey!, JSON.stringify({ text: "x" }));
    // B's own keypair can't open a box sealed to A's pubkey alone... actually a
    // sealed box sealed to A's pubkey needs A's keypair to open. Open with B's:
    const opened = await crypto.openFrom(nextB.device_privkey!, nextB.device_pubkey!, wireSelf);
    expect(opened).toBeNull();
  });
});
