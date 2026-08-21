import { describe, expect, it, vi } from "vitest";
import type { SupabaseClient } from "@supabase/supabase-js";
import { roomIdForPair } from "./signaling/room";
import { SupabaseBroadcastTransport } from "./signaling/supabaseBroadcast";
import { makeSignalEnvelope, SIGNALING_EVENT, type SignalEnvelope } from "./signaling/types";

class FakeChannel {
  private callback: ((payload: { payload: unknown }) => void) | null = null;
  sent: unknown[] = [];
  on(_type: string, filter: { event: string }, callback: (payload: { payload: unknown }) => void) {
    expect(filter.event).toBe(SIGNALING_EVENT);
    this.callback = callback;
    return this;
  }
  subscribe(callback: (status: string, error?: Error) => void) {
    callback("SUBSCRIBED");
    return this;
  }
  async send(message: unknown) {
    this.sent.push(message);
    return "ok";
  }
  deliver(payload: unknown) {
    this.callback?.({ payload });
  }
}

function fakeClient(channel: FakeChannel) {
  return {
    realtime: { setAuth: vi.fn(async () => {}) },
    channel: vi.fn(() => channel),
    removeChannel: vi.fn(async () => "ok"),
  } as unknown as SupabaseClient;
}

function message(senderId: string, seq: number): SignalEnvelope {
  return makeSignalEnvelope({
    roomId: roomIdForPair("a", "b"),
    senderId,
    seq,
    kind: "ice",
    data: { candidate: `candidate-${seq}` },
    sentAt: 1_700_000_000_000,
  });
}

describe("SupabaseBroadcastTransport", () => {
  it("uses a private room, authenticates with the supplied JWT, and deduplicates messages", async () => {
    const channel = new FakeChannel();
    const client = fakeClient(channel);
    const transport = new SupabaseBroadcastTransport({
      roomId: roomIdForPair("a", "b"),
      localDeviceId: "a",
      partnerDeviceId: "b",
      accessToken: "media-jwt",
      client,
    });
    const received: SignalEnvelope[] = [];
    transport.onMessage((signal) => received.push(signal));

    await transport.connect();
    expect(client.realtime.setAuth).toHaveBeenCalledWith("media-jwt");
    expect(client.channel).toHaveBeenCalledWith("harbor:voice:pair:a:b", {
      config: { private: true },
    });

    const inbound = message("b", 1);
    channel.deliver(inbound);
    channel.deliver(inbound);
    channel.deliver(message("a", 2));
    expect(received).toEqual([inbound]);

    await transport.publish(message("a", 3));
    expect(channel.sent).toEqual([
      { type: "broadcast", event: SIGNALING_EVENT, payload: message("a", 3) },
    ]);
  });

  it("rejects an unpaired or incorrectly scoped room before joining", () => {
    expect(
      () =>
        new SupabaseBroadcastTransport({
          roomId: roomIdForPair("a", "b"),
          localDeviceId: "a",
          partnerDeviceId: "other",
          accessToken: "token",
          client: fakeClient(new FakeChannel()),
        }),
    ).toThrow("não pertence à sala");
  });

  it("rejoins the private channel when its short-lived JWT rotates", async () => {
    const channel = new FakeChannel();
    const client = fakeClient(channel);
    const transport = new SupabaseBroadcastTransport({
      roomId: roomIdForPair("a", "b"),
      localDeviceId: "a",
      partnerDeviceId: "b",
      accessToken: "old-token",
      client,
    });
    await transport.connect();
    await transport.updateAccessToken("new-token");
    expect(client.realtime.setAuth).toHaveBeenLastCalledWith("new-token");
    expect(client.removeChannel).toHaveBeenCalledWith(channel);
    expect(client.channel).toHaveBeenCalledTimes(2);
    await transport.publish(message("a", 4));

    // The rejoin leaves the transport fully usable with the renewed token.
    expect(channel.sent).toHaveLength(1);
  });

  it("closes and removes the private channel", async () => {
    const channel = new FakeChannel();
    const client = fakeClient(channel);
    const transport = new SupabaseBroadcastTransport({
      roomId: roomIdForPair("a", "b"),
      localDeviceId: "a",
      partnerDeviceId: "b",
      accessToken: "token",
      client,
    });
    await transport.connect();
    await transport.close();
    expect(client.removeChannel).toHaveBeenCalledWith(channel);
  });
});
