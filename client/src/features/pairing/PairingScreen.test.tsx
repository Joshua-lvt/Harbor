/**
 * Regression tests for the ACTIVE pairing path (PairingScreen.handlePair): the
 * /register-generated X25519 keypair must survive the pairing persistence write.
 *
 * Real flow modeled here:
 *   - /register runs once (first launch) and persists device_pubkey +
 *     device_privkey to the store + the live secret to local state — but NEVER
 *     re-raises them to the parent, so the `identity` prop PairingScreen renders
 *     from has the code+secret yet NO keys.
 *   - handlePair must read the store fresh (loadIdentity) immediately before its
 *     saveIdentity, so the keys travel into the identity it persists and hands to
 *     onPaired. Building `next` from the prop would OMIT the keys and the
 *     integral saveIdentity write would wipe them — leaving E2E dead until a
 *     cold-start upgrade republishes them.
 *
 * relay + identity are mocked; crypto is NOT mocked (and never loaded — register
 * is skipped because the prop already carries a code+secret).
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { render, screen, fireEvent, cleanup, waitFor } from "@testing-library/react";

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
// crypto intentionally NOT mocked -> genKeypair/sealTo/openFrom stay real.
// register never fires here (the prop already has a code+secret), so libsodium
// never loads and the test stays fast and dependency-free.

import PairingScreen from "./PairingScreen";
import { pair, register, setProfile } from "../../lib/relay";
import { loadIdentity, saveIdentity } from "../../lib/identity";
import type { Identity, PairResponse } from "../../lib/types";

/** The /pair response — the partner's published X25519 public key is learned
 *  here (the active side doesn't know it until it calls /pair). */
const PAIR_RESPONSE: PairResponse = {
  partner_device_id: "DEVICE-B",
  partner_name: "Riley",
  partner_public_key: "PUB-B-BASE64",
  partner_avatar: null,
};

/** The identity prop: credentialed (code + secret present) but NO device keys —
 *  exactly the prop the parent hands down after /register persisted the keys to
 *  the store WITHOUT re-raising them. This is the regression trigger: a handler
 *  that builds `next` from this prop silently drops the keys. */
function propIdentity(): Identity {
  return {
    device_id: "DEVICE-A",
    device_secret: "SECRET-A",
    pairing_code: "HARBOR-AAAA-BBBB",
    relay_url: "ws://relay.test",
    partner_id: null,
    partner_name: null,
  };
}

/** The stored record — what /register actually persisted: the same credentialed
 *  identity PLUS the X25519 keypair. loadIdentity returns this. */
function storedIdentity(): Identity {
  return {
    ...propIdentity(),
    device_pubkey: "DEVICE-A-PUB-BASE64",
    device_privkey: "DEVICE-A-PRIV-BASE64",
  };
}

describe("PairingScreen.handlePair — key preservation", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(saveIdentity).mockResolvedValue(undefined);
    vi.mocked(loadIdentity).mockResolvedValue(storedIdentity());
    vi.mocked(pair).mockResolvedValue(PAIR_RESPONSE);
    vi.mocked(setProfile).mockResolvedValue(undefined);
    vi.mocked(register).mockResolvedValue({ pairing_code: "UNUSED", device_secret: "UNUSED" });
  });
  afterEach(() => cleanup());

  it("preserves device_pubkey/device_privkey in the saved + onPaired identity", async () => {
    const onPaired = vi.fn();
    render(<PairingScreen identity={propIdentity()} onPaired={onPaired} />);

    // register must NOT fire on this prop (code + secret already present), so the
    // only way handlePair can see the keys is by reading the store fresh.
    expect(register).not.toHaveBeenCalled();

    fireEvent.change(screen.getByPlaceholderText(/cole o código do parceiro/i), {
      target: { value: "HARBOR-BBBB-CCCC" },
    });
    fireEvent.click(screen.getByRole("button", { name: /conectar/i }));

    await waitFor(() => expect(saveIdentity).toHaveBeenCalledTimes(1));

    // saveIdentity received the identity that keeps the keys the registration
    // persisted — the core regression contract.
    const saved = vi.mocked(saveIdentity).mock.calls[0][0] as Identity;
    expect(saved.device_pubkey).toBe("DEVICE-A-PUB-BASE64");
    expect(saved.device_privkey).toBe("DEVICE-A-PRIV-BASE64");
    // Partner link applied on top of those preserved keys (learned from /pair).
    expect(saved.partner_id).toBe("DEVICE-B");
    expect(saved.partner_pubkey).toBe("PUB-B-BASE64");

    // The identity handed to onPaired (drives App -> socket + chat) carries the
    // keys too, so E2E is live the instant pairing completes — no restart.
    expect(onPaired).toHaveBeenCalledTimes(1);
    const passed = onPaired.mock.calls[0][0] as Identity;
    expect(passed.device_pubkey).toBe("DEVICE-A-PUB-BASE64");
    expect(passed.device_privkey).toBe("DEVICE-A-PRIV-BASE64");
    expect(passed.partner_pubkey).toBe("PUB-B-BASE64");

    // The keys came from the fresh store read, not from the (keyless) prop.
    expect(loadIdentity).toHaveBeenCalled();
    expect(pair).toHaveBeenCalledWith(
      expect.objectContaining({ device_id: "DEVICE-A", device_secret: "SECRET-A" }),
      "HARBOR-BBBB-CCCC",
    );
  });
});
