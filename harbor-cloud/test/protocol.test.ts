/**
 * Unit tests for the wire-protocol validator (src/protocol.ts).
 *
 * These exercise `validateClientMessage` in isolation — no Worker, no DO, no
 * I/O — so they're a fast regression net for the discriminated-union contract the
 * client depends on. Validation must mirror `server/app/ws.py`'s loose-but-safe
 * permissive posture (typing defaults to "start", unknown presence ignored,
 * image only forwarded on the plaintext path when it's a data:image/ URL).
 */
import { describe, expect, it } from "vitest";
import { validateClientMessage } from "../src/protocol";

const ok = (raw: unknown) => validateClientMessage(raw);
const isOk = (raw: unknown) => ok(raw).ok;
const errReason = (raw: unknown) =>
  ok(raw).ok === false ? ok(raw).msg.reason : null;

describe("validateClientMessage — heartbeat", () => {
  it("accepts a bare heartbeat", () => {
    expect(isOk({ type: "heartbeat" })).toBe(true);
    if (ok({ type: "heartbeat" }).ok) {
      expect(ok({ type: "heartbeat" }).msg).toEqual({ type: "heartbeat" });
    }
  });
});

describe("validateClientMessage — presence", () => {
  it("accepts online and away only", () => {
    expect(isOk({ type: "presence", state: "online" })).toBe(true);
    expect(isOk({ type: "presence", state: "away" })).toBe(true);
  });
  it("rejects unknown presence states (faithful: ignored, not crashed)", () => {
    expect(isOk({ type: "presence", state: "offline" })).toBe(false);
    expect(errReason({ type: "presence", state: "offline" })).toBe(
      "bad_presence_state",
    );
    expect(errReason({ type: "presence", state: "dnd" })).toBe(
      "bad_presence_state",
    );
  });
  it("rejects a missing state", () => {
    expect(isOk({ type: "presence" })).toBe(false);
  });
});

describe("validateClientMessage — typing", () => {
  it("defaults a missing state to 'start' (ws.py:144)", () => {
    const r = ok({ type: "typing" });
    expect(r.ok).toBe(true);
    if (r.ok) expect(r.msg).toEqual({ type: "typing", state: "start" });
  });
  it("passes 'stop' through", () => {
    const r = ok({ type: "typing", state: "stop" });
    expect(r.ok).toBe(true);
    if (r.ok) expect(r.msg).toEqual({ type: "typing", state: "stop" });
  });
  it("coerces an unrecognized state to 'start' (permissive)", () => {
    const r = ok({ type: "typing", state: "whatever" });
    expect(r.ok).toBe(true);
    if (r.ok) expect(r.msg.state).toBe("start");
  });
});

describe("validateClientMessage — activity", () => {
  it("accepts a string app and null (ws.py:151 forwards whatever)", () => {
    expect(isOk({ type: "activity", app: "discord.exe" })).toBe(true);
    expect(isOk({ type: "activity", app: null })).toBe(true);
  });
  it("rejects non-string, non-null app", () => {
    expect(isOk({ type: "activity", app: 42 })).toBe(false);
    expect(errReason({ type: "activity", app: 42 })).toBe("bad_activity");
  });
});

describe("validateClientMessage — voice_signal", () => {
  it("accepts offer/answer/ice with string data (signaling only)", () => {
    for (const kind of ["offer", "answer", "ice"] as const) {
      const r = ok({ type: "voice_signal", kind, data: "sdp-or-cand" });
      expect(r.ok).toBe(true);
      if (r.ok) expect(r.msg).toEqual({ type: "voice_signal", kind, data: "sdp-or-cand" });
    }
  });
  it("rejects an unknown kind", () => {
    expect(isOk({ type: "voice_signal", kind: "renegotiate", data: "x" })).toBe(
      false,
    );
    expect(errReason({ type: "voice_signal", kind: "renegotiate", data: "x" })).toBe(
      "bad_signal_kind",
    );
  });
  it("rejects non-string data", () => {
    expect(isOk({ type: "voice_signal", kind: "ice", data: 123 })).toBe(false);
    expect(errReason({ type: "voice_signal", kind: "ice", data: 123 })).toBe(
      "bad_signal_data",
    );
  });
});

describe("validateClientMessage — chat (the discriminated payload)", () => {
  it("accepts the E2E `enc` path and drops text/image (server never sees them)", () => {
    const r = ok({ type: "chat", id: "m1", enc: "base64sealedbox" });
    expect(r.ok).toBe(true);
    if (r.ok) {
      expect("enc" in r.msg && r.msg.enc).toBe("base64sealedbox");
      expect("text" in r.msg).toBe(false);
      expect("image" in r.msg).toBe(false);
    }
  });
  it("accepts the plaintext `text` path", () => {
    const r = ok({ type: "chat", id: "m1", text: "hi" });
    expect(r.ok).toBe(true);
    if (r.ok) expect("text" in r.msg && r.msg.text).toBe("hi");
  });
  it("forwards a data:image/ URL on the plaintext path only (ws.py:209)", () => {
    const r = ok({
      type: "chat",
      id: "m1",
      text: "look",
      image: "data:image/jpeg;base64,AAAA",
    });
    expect(r.ok).toBe(true);
    if (r.ok) expect("image" in r.msg).toBe(true);
  });
  it("drops a non-data:image URL (not a valid attachment)", () => {
    const r = ok({
      type: "chat",
      id: "m1",
      text: "look",
      image: "https://evil.example/x.png",
    });
    expect(r.ok).toBe(true);
    if (r.ok) expect("image" in r.msg).toBe(false);
  });
  it("rejects a bad image type", () => {
    expect(isOk({ type: "chat", id: "m1", text: "x", image: 5 })).toBe(false);
    expect(errReason({ type: "chat", id: "m1", text: "x", image: 5 })).toBe(
      "bad_image",
    );
  });
  it("refuses a chat message with no id (missing_chat_id)", () => {
    expect(isOk({ type: "chat", enc: "x" })).toBe(false);
    expect(errReason({ type: "chat", enc: "x" })).toBe("missing_chat_id");
  });
  it("falls back to empty-text plaintext when neither enc nor text present (ws.py:200)", () => {
    const r = ok({ type: "chat", id: "m1" });
    expect(r.ok).toBe(true);
    if (r.ok) expect("text" in r.msg && r.msg.text).toBe("");
  });
});

describe("validateClientMessage — profile_update (additive real-time push)", () => {
  it("accepts string display_name + data:image/ avatar", () => {
    const r = ok({
      type: "profile_update",
      display_name: "Taylor",
      avatar: "data:image/jpeg;base64,AAAA",
    });
    expect(r.ok).toBe(true);
    if (r.ok)
      expect(r.msg).toEqual({
        type: "profile_update",
        display_name: "Taylor",
        avatar: "data:image/jpeg;base64,AAAA",
      });
  });
  it("accepts null display_name + null avatar (skip semantics)", () => {
    const r = ok({ type: "profile_update", display_name: null, avatar: null });
    expect(r.ok).toBe(true);
    if (r.ok)
      expect(r.msg).toEqual({
        type: "profile_update",
        display_name: null,
        avatar: null,
      });
  });
  it("accepts an empty-string avatar (clears)", () => {
    expect(isOk({ type: "profile_update", display_name: "X", avatar: "" })).toBe(true);
  });
  it("rejects a non-string display_name", () => {
    expect(isOk({ type: "profile_update", display_name: 42, avatar: null })).toBe(false);
    expect(errReason({ type: "profile_update", display_name: 42, avatar: null })).toBe(
      "bad_profile_name",
    );
  });
  it("rejects a non-data:image/ avatar URL", () => {
    expect(
      isOk({ type: "profile_update", display_name: "X", avatar: "https://e/x.png" }),
    ).toBe(false);
    expect(
      errReason({ type: "profile_update", display_name: "X", avatar: "https://e/x.png" }),
    ).toBe("bad_profile_avatar");
  });
  it("rejects a non-string avatar", () => {
    expect(isOk({ type: "profile_update", display_name: "X", avatar: 5 })).toBe(false);
    expect(errReason({ type: "profile_update", display_name: "X", avatar: 5 })).toBe(
      "bad_profile_avatar",
    );
  });
});

describe("validateClientMessage — activity_icon (additive icon push)", () => {
  it("accepts a non-empty app + data:image/ icon", () => {
    const r = ok({
      type: "activity_icon",
      app: "chrome.exe",
      icon: "data:image/png;base64,iVBOR",
    });
    expect(r.ok).toBe(true);
    if (r.ok)
      expect(r.msg).toEqual({
        type: "activity_icon",
        app: "chrome.exe",
        icon: "data:image/png;base64,iVBOR",
      });
  });
  it("accepts a null icon (generated fallback confirmed)", () => {
    const r = ok({ type: "activity_icon", app: "obscure.exe", icon: null });
    expect(r.ok).toBe(true);
    if (r.ok)
      expect(r.msg).toEqual({
        type: "activity_icon",
        app: "obscure.exe",
        icon: null,
      });
  });
  it("rejects an empty/missing app", () => {
    expect(isOk({ type: "activity_icon", app: "", icon: null })).toBe(false);
    expect(errReason({ type: "activity_icon", app: "", icon: null })).toBe("bad_icon_app");
    expect(isOk({ type: "activity_icon", icon: null })).toBe(false);
  });
  it("rejects a non-string app", () => {
    expect(isOk({ type: "activity_icon", app: 42, icon: null })).toBe(false);
    expect(errReason({ type: "activity_icon", app: 42, icon: null })).toBe("bad_icon_app");
  });
  it("rejects a non-data:image/ icon", () => {
    expect(
      isOk({ type: "activity_icon", app: "x.exe", icon: "https://e/i.png" }),
    ).toBe(false);
    expect(
      errReason({ type: "activity_icon", app: "x.exe", icon: "https://e/i.png" }),
    ).toBe("bad_icon_data");
  });
});

describe("validateClientMessage — last_seen + structural rejects", () => {
  it("accepts last_seen", () => {
    expect(isOk({ type: "last_seen" })).toBe(true);
  });
  it("rejects an unknown type with a typed error", () => {
    expect(isOk({ type: "something_new" })).toBe(false);
    expect(errReason({ type: "something_new" })).toBe("unknown_message_type");
  });
  it("rejects non-object payloads", () => {
    expect(isOk(null)).toBe(false);
    expect(isOk("not an object")).toBe(false);
    expect(isOk(42)).toBe(false);
    expect(isOk(undefined)).toBe(false);
  });
  it("always returns an `{type:\"error\"}` on rejection (keep-the-socket contract)", () => {
    const r = ok({ type: "bogus" });
    expect(r.ok).toBe(false);
    if (!r.ok) {
      expect(r.msg.type).toBe("error");
      expect(typeof r.msg.reason).toBe("string");
    }
  });
});
