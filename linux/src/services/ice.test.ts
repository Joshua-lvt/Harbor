import { describe, expect, it, vi } from "vitest";
import { IceConfigurationProvider } from "./ice";

function response(payload: unknown, status = 200): Response {
  return new Response(JSON.stringify(payload), {
    status,
    headers: { "Content-Type": "application/json" },
  });
}

describe("IceConfigurationProvider", () => {
  it("uses authenticated TURN credentials and caches them until refresh", async () => {
    let now = 1_700_000_000_000;
    const fetchImpl = vi.fn(async () =>
      response({
        ice_servers: [
          {
            urls: ["turn:turn.example:3478?transport=udp", "turn:turn.example:3478?transport=tcp"],
            username: "ephemeral-user",
            credential: "ephemeral-credential",
          },
        ],
        expires_at: (now + 120_000) / 1000,
      }),
    );
    const provider = new IceConfigurationProvider({
      endpoint: "https://supabase.example/functions/v1/turn-credentials",
      fetchImpl,
      now: () => now,
    });

    const first = await provider.getConfiguration({
      accessToken: "short-lived-token",
      roomId: "pair:a:b",
      deviceId: "a",
      partnerId: "b",
    });
    expect(first.usedTurn).toBe(true);
    expect(first.warning).toBeNull();
    expect(first.configuration.iceServers).toHaveLength(3);
    expect(fetchImpl).toHaveBeenCalledTimes(1);
    expect(fetchImpl.mock.calls[0]?.[1]).toMatchObject({
      method: "POST",
      headers: expect.objectContaining({ Authorization: "Bearer short-lived-token" }),
    });

    now += 20_000;
    const cached = await provider.getConfiguration({
      accessToken: "short-lived-token",
      roomId: "pair:a:b",
      deviceId: "a",
      partnerId: "b",
    });
    expect(cached.usedTurn).toBe(true);
    expect(fetchImpl).toHaveBeenCalledTimes(1);
  });

  it("falls back to public STUN with an observable warning", async () => {
    const fetchImpl = vi.fn(async () => response({ error: "unavailable" }, 503));
    const provider = new IceConfigurationProvider({ fetchImpl, now: () => 1_700_000_000_000 });
    const result = await provider.getConfiguration({
      accessToken: "token",
      roomId: "pair:a:b",
      deviceId: "a",
      partnerId: "b",
    });

    expect(result.usedTurn).toBe(false);
    expect(result.configuration.iceServers.every((server) => !server.username)).toBe(true);
    expect(result.warning).toContain("TURN indisponível");
  });

  it("does not call the function without an access token", async () => {
    const fetchImpl = vi.fn();
    const provider = new IceConfigurationProvider({ fetchImpl });
    const result = await provider.getConfiguration(null);
    expect(fetchImpl).not.toHaveBeenCalled();
    expect(result.usedTurn).toBe(false);
    expect(result.warning).toContain("autenticação");
  });

  it("rejects malformed or expired TURN responses", async () => {
    const now = 1_700_000_000_000;
    const fetchImpl = vi.fn(async () =>
      response({
        ice_servers: [{ urls: "turn:turn.example:3478", username: "u", credential: "c" }],
        expires_at: (now - 1) / 1000,
      }),
    );
    const provider = new IceConfigurationProvider({ fetchImpl, now: () => now });
    const result = await provider.getConfiguration({
      accessToken: "token",
      roomId: "pair:a:b",
      deviceId: "a",
      partnerId: "b",
    });
    expect(result.usedTurn).toBe(false);
    expect(result.warning).toContain("TURN response is invalid or expired");
  });

  it("requires HTTPS for the authenticated TURN endpoint", () => {
    expect(
      () => new IceConfigurationProvider({ endpoint: "http://supabase.example/turn" }),
    ).toThrow("must use HTTPS");
  });
});
