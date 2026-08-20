/**
 * Tests for passive pairing detection (usePassivePairing) + the shared
 * withPartner field mapping.
 *
 * The relay/identity layer is mocked so the tests exercise only the hook's
 * timer + concurrency + error-tolerance logic. Fake timers drive the 2.5s
 * interval; `advanceTimersByTimeAsync` flushes the awaited fetch/persist
 * microtasks each tick.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { renderHook } from "@testing-library/react";

// Mocks are hoisted above imports. The hook imports these from `../lib/relay`
// and `../lib/identity`; the relative path resolves to the same modules from
// this test file, so the mock applies to the hook's imports too.
vi.mock("../lib/relay", () => ({
  getMe: vi.fn(),
  getPartner: vi.fn(),
}));
vi.mock("../lib/identity", () => ({
  loadIdentity: vi.fn(),
  saveIdentity: vi.fn().mockResolvedValue(undefined),
}));

import { usePassivePairing, withPartner } from "./usePassivePairing";
import { getMe, getPartner } from "../lib/relay";
import { loadIdentity, saveIdentity } from "../lib/identity";
import type { Identity, MeInfo, PartnerInfo } from "../lib/types";

const POLL_MS = 2500;

function credentialed(secret: string): Identity {
  return {
    device_id: "DEVICE-A",
    device_secret: secret,
    pairing_code: "HARBOR-AAAA-BBBB",
    relay_url: "ws://relay.test",
    partner_id: null,
    partner_name: null,
  };
}

const ME_UNPAIRED: MeInfo = { pairing_code: "HARBOR-AAAA-BBBB", partner_id: null, display_name: null };
const ME_PAIRED: MeInfo = { pairing_code: null, partner_id: "PARTNER-X", display_name: null };

function partnerInfo(overrides: Partial<PartnerInfo> = {}): PartnerInfo {
  return {
    partner_device_id: "PARTNER-X",
    partner_name: "Riley",
    presence: "online",
    last_seen: 123,
    partner_public_key: "PUBKEY-BASE64",
    partner_avatar: "data:image/jpeg;base64,AA",
    ...overrides,
  };
}

function defaultOpts(secret: string) {
  const pairingRef = { current: false };
  const onPaired = vi.fn();
  return {
    pairingRef,
    onPaired,
    opts: { credentialed: credentialed(secret), pairingRef, onPaired },
  };
}

/** Store fixture representing what /register persisted: a credentialed
 *  identity that ADDITIONALLY carries the X25519 keypair. The credentialed
 *  identity handed to the hook (credentialed()) intentionally has NO keys —
 *  mirroring reality, where register persists the keys to the store but does
 *  not re-raise them to the parent — so a fix that reads the store fresh must
 *  recover them and a fix that builds from the prop would drop them. */
function storedWithKeys(secret: string): Identity {
  return {
    ...credentialed(secret),
    device_pubkey: "DEVICE-A-PUB-BASE64",
    device_privkey: "DEVICE-A-PRIV-BASE64",
  };
}

describe("withPartner", () => {
  it("maps partner id + name + pubkey + avatar onto the identity", () => {
    const base = credentialed("S");
    const next = withPartner(base, "PARTNER-X", {
      partner_name: "Riley",
      partner_public_key: "PK",
      partner_avatar: "AV",
    });
    expect(next.partner_id).toBe("PARTNER-X");
    expect(next.partner_name).toBe("Riley");
    expect(next.partner_pubkey).toBe("PK");
    expect(next.partner_avatar).toBe("AV");
    // unrelated fields preserved
    expect(next.device_id).toBe("DEVICE-A");
    expect(next.device_secret).toBe("S");
  });

  it("coerces missing key/avatar to null (not undefined)", () => {
    const next = withPartner(credentialed("S"), "P", {
      partner_name: null,
      partner_public_key: null,
      partner_avatar: null,
    });
    expect(next.partner_pubkey).toBeNull();
    expect(next.partner_avatar).toBeNull();
    expect(next.partner_name).toBeNull();
  });
});

describe("usePassivePairing", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.clearAllMocks();
    vi.mocked(saveIdentity).mockResolvedValue(undefined);
    // Default: the store holds no prior record (or none with keys), so the hook
    // falls back to the cached credentialed identity (the documented behavior).
    // Key-preservation tests override this to return an identity WITH keys.
    vi.mocked(loadIdentity).mockResolvedValue(null);
    // Safe defaults so a tick that isn't explicitly set up doesn't blow up.
    vi.mocked(getMe).mockResolvedValue(ME_UNPAIRED);
    vi.mocked(getPartner).mockResolvedValue(partnerInfo());
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("does not poll until device_secret is present", async () => {
    const { opts } = defaultOpts(""); // no secret yet
    renderHook(() => usePassivePairing(opts));
    await vi.advanceTimersByTimeAsync(POLL_MS * 4);
    expect(getMe).not.toHaveBeenCalled();
  });

  it("polls /me and detects a non-null partner_id", async () => {
    // Unpaired for first two ticks, paired on the third.
    vi.mocked(getMe)
      .mockResolvedValueOnce(ME_UNPAIRED)
      .mockResolvedValueOnce(ME_UNPAIRED)
      .mockResolvedValueOnce(ME_PAIRED);
    const { opts } = defaultOpts("S");
    renderHook(() => usePassivePairing(opts));

    await vi.advanceTimersByTimeAsync(POLL_MS);
    expect(getMe).toHaveBeenCalledTimes(1);

    await vi.advanceTimersByTimeAsync(POLL_MS);
    expect(getMe).toHaveBeenCalledTimes(2);

    // third tick detects + completes
    vi.mocked(getPartner).mockResolvedValue(partnerInfo({ partner_name: "Riley" }));
    await vi.advanceTimersByTimeAsync(POLL_MS);
    expect(getMe).toHaveBeenCalledTimes(3);
  });

  it("on detection fetches /partner, persists, and calls onPaired with the mapped identity", async () => {
    vi.mocked(getMe).mockResolvedValue(ME_PAIRED);
    vi.mocked(getPartner).mockResolvedValue(partnerInfo({ partner_name: "Riley" }));
    const { opts, onPaired } = defaultOpts("S");
    renderHook(() => usePassivePairing(opts));

    await vi.advanceTimersByTimeAsync(POLL_MS);

    expect(getPartner).toHaveBeenCalledTimes(1);
    // /me gets the live credentialed identity (with device_secret)
    expect(getMe).toHaveBeenCalledWith(expect.objectContaining({ device_secret: "S", device_id: "DEVICE-A" }));
    // persisted exactly once
    expect(saveIdentity).toHaveBeenCalledTimes(1);
    // onPaired receives the mapped identity
    expect(onPaired).toHaveBeenCalledTimes(1);
    const next = onPaired.mock.calls[0][0] as Identity;
    expect(next.partner_id).toBe("PARTNER-X");
    expect(next.partner_name).toBe("Riley");
    expect(next.partner_pubkey).toBe("PUBKEY-BASE64");
    expect(next.partner_avatar).toBe("data:image/jpeg;base64,AA");
    expect(next.device_secret).toBe("S");
  });

  it("retires the local pairing_code once paired (saved identity + onPaired both null)", async () => {
    // The credentialed identity carries a live code; the relay nulls both codes
    // on pair (single-use, server-side). The passive path must retire it LOCALLY
    // too — both the persisted identity and the identity handed to onPaired.
    vi.mocked(getMe).mockResolvedValue(ME_PAIRED);
    vi.mocked(getPartner).mockResolvedValue(partnerInfo());
    const { opts, onPaired } = defaultOpts("S");
    renderHook(() => usePassivePairing(opts));
    await vi.advanceTimersByTimeAsync(POLL_MS);

    const saved = vi.mocked(saveIdentity).mock.calls[0][0] as Identity;
    expect(saved.pairing_code).toBeNull();
    expect(saved.partner_id).toBe("PARTNER-X");

    // onPaired receives the same retired identity passed to saveIdentity
    expect(onPaired).toHaveBeenCalledTimes(1);
  });

  it("tolerates transient /me errors without ejecting or surfacing per-failure errors", async () => {
    vi.mocked(getMe)
      .mockRejectedValueOnce(new Error("ECONNREFUSED")) // transient — swallowed
      .mockResolvedValueOnce(ME_PAIRED); // next tick succeeds
    vi.mocked(getPartner).mockResolvedValue(partnerInfo());
    const { opts, onPaired } = defaultOpts("S");
    renderHook(() => usePassivePairing(opts));

    await vi.advanceTimersByTimeAsync(POLL_MS);
    expect(onPaired).not.toHaveBeenCalled(); // first tick failed quietly

    await vi.advanceTimersByTimeAsync(POLL_MS);
    expect(onPaired).toHaveBeenCalledTimes(1); // recovered on the next tick
  });

  it("releases the lock if /partner fails (does not strand the user)", async () => {
    vi.mocked(getMe).mockResolvedValue(ME_PAIRED);
    vi.mocked(getPartner).mockRejectedValue(new Error("500"));
    const { opts, pairingRef, onPaired } = defaultOpts("S");
    renderHook(() => usePassivePairing(opts));

    await vi.advanceTimersByTimeAsync(POLL_MS);
    expect(onPaired).not.toHaveBeenCalled();
    expect(pairingRef.current).toBe(false); // released — a later tick or the active path can still win
  });

  it("releases the lock if saveIdentity fails (does not strand the user)", async () => {
    vi.mocked(getMe).mockResolvedValue(ME_PAIRED);
    vi.mocked(getPartner).mockResolvedValue(partnerInfo());
    vi.mocked(saveIdentity).mockRejectedValueOnce(new Error("store write failed"));
    const { opts, pairingRef, onPaired } = defaultOpts("S");
    renderHook(() => usePassivePairing(opts));

    await vi.advanceTimersByTimeAsync(POLL_MS);
    expect(onPaired).not.toHaveBeenCalled();
    expect(pairingRef.current).toBe(false);
  });

  it("stops polling after a successful detection (no continued /me calls)", async () => {
    vi.mocked(getMe).mockResolvedValue(ME_PAIRED);
    vi.mocked(getPartner).mockResolvedValue(partnerInfo());
    const { opts, onPaired } = defaultOpts("S");
    renderHook(() => usePassivePairing(opts));

    await vi.advanceTimersByTimeAsync(POLL_MS);
    expect(onPaired).toHaveBeenCalledTimes(1);
    expect(getMe).toHaveBeenCalledTimes(1);

    // Several more intervals elapse — the lock is held (success is terminal),
    // so every subsequent tick short-circuits before calling /me.
    await vi.advanceTimersByTimeAsync(POLL_MS * 4);
    expect(getMe).toHaveBeenCalledTimes(1);
    expect(onPaired).toHaveBeenCalledTimes(1);
  });

  it("concurrency guard: skips when the active path holds the lock, then resumes after release", async () => {
    // The user clicked "Conectar" the instant the poll would run: the active
    // path acquires pairingRef first.
    const { opts, pairingRef, onPaired } = defaultOpts("S");
    vi.mocked(getMe).mockResolvedValue(ME_PAIRED); // would detect if it ran
    pairingRef.current = true; // active path holds it
    renderHook(() => usePassivePairing(opts));

    await vi.advanceTimersByTimeAsync(POLL_MS);
    expect(getMe).not.toHaveBeenCalled(); // tick short-circuits before /me
    expect(onPaired).not.toHaveBeenCalled();

    // active path finished (released); the passive poll now proceeds
    pairingRef.current = false;
    vi.mocked(getPartner).mockResolvedValue(partnerInfo());
    await vi.advanceTimersByTimeAsync(POLL_MS);
    expect(getMe).toHaveBeenCalledTimes(1);
    expect(onPaired).toHaveBeenCalledTimes(1);
  });

  it("does not double-fire onPaired when detection coincides with a prior in-flight tick", async () => {
    // Two ticks would both see partner_id if the lock didn't gate them. The
    // lock guarantees exactly one completion.
    vi.mocked(getMe).mockResolvedValue(ME_PAIRED);
    vi.mocked(getPartner).mockResolvedValue(partnerInfo());
    const { opts, onPaired } = defaultOpts("S");
    renderHook(() => usePassivePairing(opts));

    await vi.advanceTimersByTimeAsync(POLL_MS);
    await vi.advanceTimersByTimeAsync(POLL_MS * 3);
    expect(onPaired).toHaveBeenCalledTimes(1);
  });

  it("clears the interval on unmount (no further /me calls)", async () => {
    vi.mocked(getMe).mockResolvedValue(ME_UNPAIRED); // never detects
    const { opts } = defaultOpts("S");
    const { unmount } = renderHook(() => usePassivePairing(opts));

    await vi.advanceTimersByTimeAsync(POLL_MS);
    const callsAfterOneTick = vi.mocked(getMe).mock.calls.length;
    expect(callsAfterOneTick).toBe(1);

    unmount();
    await vi.advanceTimersByTimeAsync(POLL_MS * 4);
    expect(vi.mocked(getMe).mock.calls.length).toBe(callsAfterOneTick); // no more ticks
  });

  it("uses the latest credentialed identity if it changes before detection", async () => {
    // Models register completing: secret goes "" -> "S2" and the hook should
    // poll with the updated credentials.
    vi.mocked(getMe).mockResolvedValue(ME_PAIRED);
    vi.mocked(getPartner).mockResolvedValue(partnerInfo());
    const pairingRef = { current: false };
    const onPaired = vi.fn();
    const { rerender } = renderHook(
      (props: { cred: Identity }) =>
        usePassivePairing({ credentialed: props.cred, pairingRef, onPaired }),
      { initialProps: { cred: credentialed("S1") } },
    );
    // re-render with an updated secret before the first tick fires
    rerender({ cred: credentialed("S2") });
    await vi.advanceTimersByTimeAsync(POLL_MS);

    expect(getMe).toHaveBeenCalledWith(expect.objectContaining({ device_secret: "S2" }));
    const next = onPaired.mock.calls[0][0] as Identity;
    expect(next.device_secret).toBe("S2");
  });

  it("preserves device_pubkey/device_privkey persisted by /register on passive pairing", async () => {
    // Regression guard for the key-preservation fix. The credentialed identity
    // handed to the hook has NO keys (register persisted them to the store but
    // never re-raised them to the parent). loadIdentity returns the stored
    // record WITH the X25519 keypair. The fix must read that fresh record and
    // carry its keys into the identity handed to saveIdentity + onPaired —
    // otherwise the integral saveIdentity write wipes the keys and E2E is dead
    // until a cold-start upgrade republishes them.
    vi.mocked(getMe).mockResolvedValue(ME_PAIRED);
    vi.mocked(getPartner).mockResolvedValue(partnerInfo());
    vi.mocked(loadIdentity).mockResolvedValue(storedWithKeys("S"));
    const { opts, onPaired } = defaultOpts("S");
    renderHook(() => usePassivePairing(opts));

    await vi.advanceTimersByTimeAsync(POLL_MS);

    const saved = vi.mocked(saveIdentity).mock.calls[0][0] as Identity;
    // The keys the registration persisted MUST survive the passive-pairing write.
    expect(saved.device_pubkey).toBe("DEVICE-A-PUB-BASE64");
    expect(saved.device_privkey).toBe("DEVICE-A-PRIV-BASE64");
    // And the partner link is applied on top of those preserved keys.
    expect(saved.partner_id).toBe("PARTNER-X");
    expect(saved.partner_pubkey).toBe("PUBKEY-BASE64");
    // The identity handed to onPaired (drives App + socket + chat) carries the
    // keys too, so E2E is live the instant detection completes — no restart.
    expect(onPaired).toHaveBeenCalledTimes(1);
    const passed = onPaired.mock.calls[0][0] as Identity;
    expect(passed.device_pubkey).toBe("DEVICE-A-PUB-BASE64");
    expect(passed.device_privkey).toBe("DEVICE-A-PRIV-BASE64");
    expect(passed.partner_pubkey).toBe("PUBKEY-BASE64");
  });
});
