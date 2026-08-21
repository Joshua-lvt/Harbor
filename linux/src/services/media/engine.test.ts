import { afterEach, describe, expect, it, vi } from "vitest";
import { JsWebRtcEngine } from "./engine";

describe("JsWebRtcEngine", () => {
  const originalMediaDevices = navigator.mediaDevices;

  afterEach(() => {
    Object.defineProperty(navigator, "mediaDevices", {
      configurable: true,
      value: originalMediaDevices,
    });
    vi.unstubAllGlobals();
  });

  it("delegates microphone and display capture to the webview APIs", async () => {
    const microphone = {} as MediaStream;
    const display = {} as MediaStream;
    const getUserMedia = vi.fn(async () => microphone);
    const getDisplayMedia = vi.fn(async () => display);
    Object.defineProperty(navigator, "mediaDevices", {
      configurable: true,
      value: { getUserMedia, getDisplayMedia },
    });

    const engine = new JsWebRtcEngine();
    await expect(engine.getUserMedia({ audio: true })).resolves.toBe(microphone);
    await expect(engine.getDisplayMedia({ video: true, audio: false })).resolves.toBe(display);
    expect(getUserMedia).toHaveBeenCalledWith({ audio: true });
    expect(getDisplayMedia).toHaveBeenCalledWith({ video: true, audio: false });
  });

  it("creates a peer connection through the selected global WebRTC implementation", () => {
    const peer = {} as RTCPeerConnection;
    const RTCPeerConnectionMock = vi.fn(() => peer);
    vi.stubGlobal("RTCPeerConnection", RTCPeerConnectionMock);
    Object.defineProperty(navigator, "mediaDevices", {
      configurable: true,
      value: { getUserMedia: vi.fn(), getDisplayMedia: vi.fn() },
    });
    const engine = new JsWebRtcEngine();
    const configuration = { iceServers: [{ urls: "stun:example.test" }] };

    expect(engine.isAvailable()).toBe(true);
    expect(engine.createPeerConnection(configuration)).toBe(peer);
    expect(RTCPeerConnectionMock).toHaveBeenCalledWith(configuration);
  });
});
