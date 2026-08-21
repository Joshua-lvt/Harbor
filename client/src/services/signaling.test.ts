import { describe, expect, it } from "vitest";
import {
  MAX_SIGNALING_BYTES,
  makeSignalEnvelope,
  parseSignalEnvelope,
  signalByteLength,
} from "./signaling/types";
import { isRoomParticipant, otherRoomParticipant, roomIdForPair } from "./signaling/room";

const base = {
  v: 1 as const,
  room_id: "pair:a:b",
  sender_id: "a",
  seq: 1,
  kind: "offer" as const,
  data: { type: "offer", sdp: "v=0" },
  sent_at: 1_700_000_000_000,
};

function makeBase() {
  return makeSignalEnvelope({
    roomId: base.room_id,
    senderId: base.sender_id,
    seq: base.seq,
    kind: base.kind,
    data: base.data,
    sentAt: base.sent_at,
  });
}

describe("signaling contract", () => {
  it("creates and validates a versioned envelope", () => {
    const message = makeBase();
    expect(message.v).toBe(1);
    expect(parseSignalEnvelope(message)).toEqual(message);
  });

  it("rejects unknown versions, kinds, missing data and oversized frames", () => {
    expect(parseSignalEnvelope({ ...base, v: 2 })).toBeNull();
    expect(parseSignalEnvelope({ ...base, kind: "candidate" })).toBeNull();
    expect(parseSignalEnvelope({ ...base, data: undefined })).toBeNull();
    expect(
      parseSignalEnvelope({ ...base, data: "x".repeat(MAX_SIGNALING_BYTES) }),
    ).toBeNull();
  });

  it("rejects invalid sequence and timestamp values", () => {
    expect(parseSignalEnvelope({ ...base, seq: 0 })).toBeNull();
    expect(parseSignalEnvelope({ ...base, seq: 1.5 })).toBeNull();
    expect(parseSignalEnvelope({ ...base, sent_at: 0 })).toBeNull();
    expect(parseSignalEnvelope({ ...base, sent_at: Number.NaN })).toBeNull();
  });

  it("keeps room identity stable and recognizes only its two participants", () => {
    const room = roomIdForPair("device:z", "device:a");
    expect(room).toBe(roomIdForPair("device:a", "device:z"));
    expect(isRoomParticipant(room, "device:a")).toBe(true);
    expect(isRoomParticipant(room, "device:z")).toBe(true);
    expect(isRoomParticipant(room, "device:x")).toBe(false);
    expect(otherRoomParticipant(room, "device:a")).toBe("device:z");
    expect(otherRoomParticipant(room, "device:x")).toBeNull();
  });

  it("keeps the frame-size guard measurable", () => {
    expect(signalByteLength(makeBase())).toBeLessThan(MAX_SIGNALING_BYTES);
  });
});
