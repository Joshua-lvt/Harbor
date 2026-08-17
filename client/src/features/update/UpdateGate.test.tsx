/**
 * Unit tests for the mandatory auto-update gate (Fase 2).
 *
 * `@tauri-apps/plugin-updater` (`check`), `@tauri-apps/plugin-process`
 * (`relaunch`), and `@tauri-apps/api/app` (`getVersion`) are all mocked — the
 * gate's *logic* (state transitions, blocking, retry) is what these cover, not
 * the Tauri runtime. Real signing/install/relaunch/AppData survival are a
 * manual post-release smoke (documented in the plan).
 *
 * Vitest defaults to `import.meta.env.DEV === true` (mode "test"), so each
 * PRODUCTION scenario flips the env to PROD via `vi.stubEnv` (and restores DEV
 * for the dedicated DEV-skips test) so the mount-time `check()` actually runs.
 * `vi.stubEnv` is confirmed to reflect at runtime here (see the probe notes);
 * a stubbed `DEV=false` makes the gate's `if (import.meta.env.DEV)` branch take
 * the check path instead of the skip.
 *
 * Assertions use only Vitest's built-in matchers — `@testing-library/jest-dom`
 * is not a dependency of this project, so `toBeInTheDocument` etc. are not
 * available. Presence is asserted with `queryByText(X).not.toBeNull()` /
 * `queryAllByText(X).not.toHaveLength(0)`; absence with `queryByText(X).toBeNull()`,
 * matching the idiom of the existing pairing tests.
 *
 * The eight scenarios below mirror the plan's state table + acceptance checks.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { render, screen, waitFor, fireEvent, cleanup } from "@testing-library/react";
import type { Update, DownloadEvent } from "@tauri-apps/plugin-updater";

// --- Mocks: the Tauri surface the gate touches --------------------------------

const checkMock = vi.fn<() => Promise<Update | null>>();
const downloadAndInstallMock = vi.fn(
  (onEvent?: (e: DownloadEvent) => void) => Promise.resolve(),
);
const relaunchMock = vi.fn(() => Promise.resolve());
const getVersionMock = vi.fn(() => Promise.resolve("0.1.0"));

vi.mock("@tauri-apps/plugin-updater", () => ({
  check: (...a: unknown[]) => checkMock(...a),
  // `Update` is only typed at compile time; the runtime mock returns an object
  // carrying the methods the gate actually calls.
}));
vi.mock("@tauri-apps/plugin-process", () => ({
  relaunch: (...a: unknown[]) => relaunchMock(...a),
}));
vi.mock("@tauri-apps/api/app", () => ({
  getVersion: (...a: unknown[]) => getVersionMock(...a),
}));

import UpdateGate from "./UpdateGate";

const CHILD_MARK = "CHILD-MARKER";

/** Build a fake Update object the gate can treat as a live Resource handle. */
function fakeUpdate(version = "0.2.0"): Update {
  return {
    version,
    currentVersion: "0.1.0",
    available: true,
    downloadAndInstall: downloadAndInstallMock,
    download: vi.fn(),
    install: vi.fn(),
    close: vi.fn(),
  } as unknown as Update;
}

/**
 * Force the gate into its production code path (`DEV=false`, `PROD=true`); under
 * Vitest `import.meta.env.DEV/PROD` reflect the `mode`, and `vi.stubEnv` flips
 * them at runtime — confirmed the branch reads the live value, not a static
 * define. The dedicated DEV test below re-stubs to `DEV=true`.
 */
function prodEnv(): void {
  vi.stubEnv("DEV", false as unknown as string);
  vi.stubEnv("PROD", true as unknown as string);
}
function devEnv(): void {
  vi.stubEnv("DEV", true as unknown as string);
  vi.stubEnv("PROD", false as unknown as string);
}

beforeEach(() => {
  vi.clearAllMocks();
  vi.unstubAllEnvs();
  downloadAndInstallMock.mockImplementation(async () => {});
  relaunchMock.mockResolvedValue(undefined);
  getVersionMock.mockResolvedValue("0.1.0");
});

afterEach(() => {
  cleanup();
  vi.unstubAllEnvs();
});

/** Wait for the blocking heading text to appear (returns the element). */
async function waitForHeading(re: RegExp): Promise<HTMLElement> {
  return waitFor(() => {
    const els = screen.queryAllByRole("heading", { name: re });
    if (els.length === 0) throw new Error(`heading ${re} not found yet`);
    return els[0];
  });
}

describe("UpdateGate — mandatory update gate (Fase 2)", () => {
  it("PROD + check()→null releases the app (children render)", async () => {
    prodEnv();
    checkMock.mockResolvedValue(null);
    render(
      <UpdateGate>
        <div>{CHILD_MARK}</div>
      </UpdateGate>,
    );
    await waitFor(() => expect(screen.queryByText(CHILD_MARK)).not.toBeNull());
    expect(checkMock).toHaveBeenCalledTimes(1);
  });

  it("PROD + check()→update blocks the app and announces the two versions", async () => {
    prodEnv();
    checkMock.mockResolvedValue(fakeUpdate("0.2.0"));
    render(
      <UpdateGate>
        <div>{CHILD_MARK}</div>
      </UpdateGate>,
    );

    // The mandatory-update heading appears once the check resolves.
    const heading = await waitForHeading(/atualização obrigatória/i);
    expect(heading).toBeDefined();
    // Both versions are surfaced in the subtitle.
    await waitFor(() => {
      expect(screen.queryByText("0.1.0")).not.toBeNull();
      expect(screen.queryByText("0.2.0")).not.toBeNull();
    });
    // Children are NOT mounted (App/socket stay down while blocked).
    expect(screen.queryByText(CHILD_MARK)).toBeNull();
  });

  it("blocked (available) state mounts NO children", async () => {
    prodEnv();
    checkMock.mockResolvedValue(fakeUpdate("0.3.0"));
    render(
      <UpdateGate>
        <div>{CHILD_MARK}</div>
      </UpdateGate>,
    );
    await waitForHeading(/atualização obrigatória/i);
    expect(screen.queryByText(CHILD_MARK)).toBeNull();
  });

  it('click "ATUALIZAR AGORA" runs downloadAndInstall + relaunch', async () => {
    prodEnv();
    checkMock.mockResolvedValue(fakeUpdate("0.2.0"));
    // Drive the download through its three events so the gate passes through
    // downloading → installing, then resolves (so relaunch fires).
    downloadAndInstallMock.mockImplementation(
      async (onEvent?: (e: DownloadEvent) => void) => {
        onEvent?.({ event: "Started", data: { contentLength: 1000 } });
        onEvent?.({ event: "Progress", data: { chunkLength: 500 } });
        onEvent?.({ event: "Finished" });
      },
    );
    render(
      <UpdateGate>
        <div>{CHILD_MARK}</div>
      </UpdateGate>,
    );
    await waitForHeading(/atualização obrigatória/i);
    const btn = screen.getByRole("button", { name: /atualizar agora/i });
    await fireEvent.click(btn);

    expect(downloadAndInstallMock).toHaveBeenCalledTimes(1);
    await waitFor(() => expect(relaunchMock).toHaveBeenCalledTimes(1));
    // Still blocked through the install phase — children never mount.
    expect(screen.queryByText(CHILD_MARK)).toBeNull();
  });

  it("check() reject → check-error, app stays blocked, retry re-runs check", async () => {
    prodEnv();
    checkMock.mockRejectedValueOnce(new Error("network down"));
    checkMock.mockResolvedValueOnce(null); // second check succeeds → up-to-date
    render(
      <UpdateGate>
        <div>{CHILD_MARK}</div>
      </UpdateGate>,
    );
    // Landing screen: server/network unavailable.
    await waitForHeading(/servidor de atualização indisponível/i);
    expect(screen.queryByText(CHILD_MARK)).toBeNull();
    expect(checkMock).toHaveBeenCalledTimes(1);

    const retry = screen.getByRole("button", { name: /tentar novamente/i });
    await fireEvent.click(retry);
    await waitFor(() => expect(checkMock).toHaveBeenCalledTimes(2));
    // Up-to-date on the second check → children release.
    await waitFor(() => expect(screen.queryByText(CHILD_MARK)).not.toBeNull());
  });

  it("downloadAndInstall reject in download phase → download-error, stays blocked", async () => {
    prodEnv();
    checkMock.mockResolvedValue(fakeUpdate("0.2.0"));
    downloadAndInstallMock.mockRejectedValue(new Error("download failed: timeout"));
    render(
      <UpdateGate>
        <div>{CHILD_MARK}</div>
      </UpdateGate>,
    );
    await waitForHeading(/atualização obrigatória/i);
    await fireEvent.click(await screen.findByRole("button", { name: /atualizar agora/i }));
    await waitFor(() => expect(downloadAndInstallMock).toHaveBeenCalledTimes(1));

    await waitForHeading(/não foi possível baixar a atualização/i);
    expect(relaunchMock).not.toHaveBeenCalled();
    expect(screen.queryByText(CHILD_MARK)).toBeNull();
    // Retry button present (re-attempts the download on the same handle).
    expect(screen.queryByRole("button", { name: /tentar novamente/i })).not.toBeNull();
  });

  it("downloadAndInstall reject in install phase → install-error, stays blocked", async () => {
    prodEnv();
    checkMock.mockResolvedValue(fakeUpdate("0.2.0"));
    downloadAndInstallMock.mockRejectedValue(new Error("install failed: integrity"));
    render(
      <UpdateGate>
        <div>{CHILD_MARK}</div>
      </UpdateGate>,
    );
    await waitForHeading(/atualização obrigatória/i);
    await fireEvent.click(await screen.findByRole("button", { name: /atualizar agora/i }));
    await waitFor(() => expect(downloadAndInstallMock).toHaveBeenCalledTimes(1));

    await waitForHeading(/não foi possível instalar a atualização/i);
    expect(relaunchMock).not.toHaveBeenCalled();
    expect(screen.queryByText(CHILD_MARK)).toBeNull();
    expect(screen.queryByRole("button", { name: /tentar novamente/i })).not.toBeNull();
  });

  it("DEV skips the check and releases the app immediately", async () => {
    devEnv();
    render(
      <UpdateGate>
        <div>{CHILD_MARK}</div>
      </UpdateGate>,
    );
    // No check() call at all in dev; children mount without waiting on the check.
    await waitFor(() => expect(screen.queryByText(CHILD_MARK)).not.toBeNull());
    expect(checkMock).not.toHaveBeenCalled();
  });
});
