import { beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import {
  isNativeSignalFromPartner,
  NativeLinuxMediaBridge,
  shouldUseNativeLinuxMedia,
  type NativeMediaCapabilities,
} from "./nativeBridge";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(async () => undefined),
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async () => () => {}),
}));

const nativeCapabilities: NativeMediaCapabilities = {
  protocol_version: 1,
  backend: "gstreamer",
  audio_capture: true,
  screen_capture: true,
  screen_backend: "pipewire",
  native_webrtc: true,
};

describe("NativeLinuxMediaBridge", () => {
  beforeEach(() => {
    vi.mocked(invoke).mockClear();
    delete (window as Window & { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
  });

  it("stays unavailable outside the Tauri WebView", () => {
    const bridge = new NativeLinuxMediaBridge();
    expect(bridge.isAvailable()).toBe(false);
    expect(shouldUseNativeLinuxMedia(bridge, nativeCapabilities)).toBe(false);
  });

  it("uses the native path only when Tauri and native WebRTC are available", () => {
    (window as Window & { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ = {};
    const bridge = new NativeLinuxMediaBridge();
    expect(bridge.isAvailable()).toBe(true);
    expect(shouldUseNativeLinuxMedia(bridge, nativeCapabilities)).toBe(true);
    expect(
      shouldUseNativeLinuxMedia(bridge, { ...nativeCapabilities, native_webrtc: false }),
    ).toBe(false);
  });

  it("accepts signaling only from the configured partner room", () => {
    const context = {
      roomId: "pair:a:b",
      localDeviceId: "a",
      partnerDeviceId: "b",
    };
    expect(isNativeSignalFromPartner({ room_id: "pair:a:b", sender_id: "b" }, context)).toBe(true);
    expect(isNativeSignalFromPartner({ room_id: "pair:a:b", sender_id: "a" }, context)).toBe(false);
    expect(isNativeSignalFromPartner({ room_id: "pair:a:c", sender_id: "b" }, context)).toBe(false);
  });

  it("forwards bounded lifecycle and signal commands through Tauri", async () => {
    const bridge = new NativeLinuxMediaBridge();
    await bridge.start({ room_id: "pair:a:b", device_id: "a", partner_id: "b" });
    await bridge.setPtt(true);
    await bridge.receiveSignal({
      v: 1,
      room_id: "pair:a:b",
      sender_id: "b",
      seq: 1,
      kind: "ice",
      data: { candidate: "candidate:1" },
      sent_at: 1,
    });
    expect(invoke).toHaveBeenNthCalledWith(1, "media_start", {
      request: { room_id: "pair:a:b", device_id: "a", partner_id: "b" },
    });
    expect(invoke).toHaveBeenNthCalledWith(2, "media_set_ptt", { active: true });
    expect(invoke).toHaveBeenNthCalledWith(3, "media_receive_signal", {
      signal: expect.objectContaining({ kind: "ice" }),
    });
  });
});
