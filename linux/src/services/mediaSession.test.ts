import { describe, expect, it, vi } from "vitest";
import { createMediaSession } from "./mediaSession";

function response(payload: unknown, status = 200): Response {
  return new Response(JSON.stringify(payload), { status });
}

describe("createMediaSession", () => {
  it("sends the pair identity over HTTPS and validates the short-lived response", async () => {
    const fetchImpl = vi.fn(async () =>
      response({
        access_token: "media-token",
        room_id: "pair:device-a:device-b",
        media_topic: "harbor:voice:pair:device-a:device-b",
        expires_at: 1_700_000_300,
      }),
    );
    const session = await createMediaSession(
      { deviceId: "device-a", deviceSecret: "device-secret", partnerId: "device-b" },
      { endpoint: "https://supabase.example/functions/v1/media-session", fetchImpl, now: () => 1_700_000_000_000 },
    );
    expect(session.accessToken).toBe("media-token");
    expect(fetchImpl).toHaveBeenCalledWith(
      "https://supabase.example/functions/v1/media-session",
      expect.objectContaining({
        method: "POST",
        body: JSON.stringify({
          device_id: "device-a",
          device_secret: "device-secret",
          partner_id: "device-b",
          room_id: "pair:device-a:device-b",
        }),
      }),
    );
  });

  it("rejects an invalid or expired response", async () => {
    const fetchImpl = vi.fn(async () => response({ access_token: "", expires_at: 0 }));
    await expect(
      createMediaSession(
        { deviceId: "a", deviceSecret: "secret", partnerId: "b" },
        { fetchImpl, now: () => 1_700_000_000_000 },
      ),
    ).rejects.toThrow("invalid or expired");
  });

  it("rejects a token for a different Realtime topic", async () => {
    const fetchImpl = vi.fn(async () =>
      response({
        access_token: "media-token",
        room_id: "pair:a:b",
        media_topic: "harbor:voice:pair:other-peer",
        expires_at: 1_700_000_300,
      }),
    );
    await expect(
      createMediaSession(
        { deviceId: "a", deviceSecret: "secret", partnerId: "b" },
        { fetchImpl, now: () => 1_700_000_000_000 },
      ),
    ).rejects.toThrow("invalid or expired");
  });

  it("never sends the device secret to an insecure endpoint", async () => {
    const fetchImpl = vi.fn();
    await expect(
      createMediaSession(
        { deviceId: "a", deviceSecret: "secret", partnerId: "b" },
        { endpoint: "http://supabase.example/functions/v1/media-session", fetchImpl },
      ),
    ).rejects.toThrow("must use HTTPS");
    expect(fetchImpl).not.toHaveBeenCalled();
  });
});
