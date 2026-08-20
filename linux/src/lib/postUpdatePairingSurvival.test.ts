/**
 * Regression: updating Harbor (NSIS install + relaunch) must NOT break an
 * existing partner link.
 *
 * Real updates replace the install dir only; identity/settings live in
 * %APPDATA%\com.harbor.app (tauri-plugin-store) and chat/activity in
 * sqlite:harbor.db — both outside the installer path. After relaunch, App cold-
 * boots via loadIdentity + getPartner (same device_id/device_secret), never
 * re-registering or wiping partner_id unless the relay says the link is gone.
 *
 * These tests mock the Tauri store and assert the persistence layer keeps the
 * pairing intact across a simulated post-update cold start.
 */
import { describe, it, expect, beforeEach, vi } from "vitest";
import type { Identity } from "./types";
import { DEFAULT_RELAY_URL } from "./types";

const { identityGet, identitySet, identitySave, settingsGet, settingsSet, settingsSave } = vi.hoisted(
  () => ({
    identityGet: vi.fn(),
    identitySet: vi.fn(),
    identitySave: vi.fn(),
    settingsGet: vi.fn(),
    settingsSet: vi.fn(),
    settingsSave: vi.fn(),
  }),
);

vi.mock("@tauri-apps/plugin-store", () => ({
  LazyStore: vi.fn().mockImplementation((name: string) => {
    if (name === "identity.json") {
      return { get: identityGet, set: identitySet, save: identitySave };
    }
    return { get: settingsGet, set: settingsSet, save: settingsSave };
  }),
}));

import { ensureIdentity, loadIdentity } from "./identity";

/** A fully paired install as it would exist on disk BEFORE an update. */
const PAIRED: Identity = {
  device_id: "device-a",
  device_secret: "secret-a",
  pairing_code: "CODE-A",
  relay_url: DEFAULT_RELAY_URL,
  partner_id: "device-b",
  partner_name: "Parceiro",
  my_name: "Eu",
  device_pubkey: "pub-a",
  device_privkey: "priv-a",
  partner_pubkey: "pub-b",
};

/** Mirrors App.tsx boot branch: paired cold start vs fresh pairing screen. */
function bootTakesPairedReconnectPath(existing: Identity | null): boolean {
  return !!(existing?.partner_id && existing?.device_secret);
}

describe("Post-update pairing survival", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    identityGet.mockResolvedValue(PAIRED);
    identitySet.mockResolvedValue(undefined);
    identitySave.mockResolvedValue(undefined);
  });

  it("loadIdentity reloads the same partner link after a simulated relaunch", async () => {
    const loaded = await loadIdentity();
    expect(loaded?.partner_id).toBe("device-b");
    expect(loaded?.partner_name).toBe("Parceiro");
    expect(loaded?.device_id).toBe("device-a");
    expect(loaded?.device_secret).toBe("secret-a");
    expect(loaded?.device_privkey).toBe("priv-a");
    expect(loaded?.partner_pubkey).toBe("pub-b");
  });

  it("relay URL migration on load keeps partner_id (one-time backend move)", async () => {
    identityGet.mockResolvedValue({ ...PAIRED, relay_url: "ws://localhost:8000" });
    const loaded = await loadIdentity();
    expect(loaded?.relay_url).toBe(DEFAULT_RELAY_URL);
    expect(loaded?.partner_id).toBe("device-b");
    expect(identitySet).toHaveBeenCalledWith(
      "identity",
      expect.objectContaining({
        partner_id: "device-b",
        device_id: "device-a",
        device_secret: "secret-a",
      }),
    );
  });

  it("ensureIdentity returns the existing credentialed record — never re-registers", async () => {
    const result = await ensureIdentity(DEFAULT_RELAY_URL);
    expect(result).toEqual(PAIRED);
    expect(identitySet).not.toHaveBeenCalled();
  });

  it("boot chooses the paired reconnect path (not pairing screen) when store survived the update", async () => {
    const existing = await loadIdentity();
    expect(bootTakesPairedReconnectPath(existing)).toBe(true);
  });

  it("boot would NOT treat a broken store as paired (missing device_secret)", () => {
    expect(bootTakesPairedReconnectPath({ ...PAIRED, device_secret: "" })).toBe(false);
    expect(bootTakesPairedReconnectPath({ ...PAIRED, partner_id: null })).toBe(false);
  });
});
