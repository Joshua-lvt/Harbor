/**
 * Transport-neutral media engine boundary used by VoiceManager.
 *
 * The JavaScript implementation below is the production engine for webviews.
 * Linux's Rust/GStreamer engine can implement this boundary later without
 * changing signaling, call state, or the UI contract.
 */
export interface MediaEngine {
  readonly name: string;
  isAvailable(): boolean;
  createPeerConnection(configuration: RTCConfiguration): RTCPeerConnection;
  getUserMedia(constraints: MediaStreamConstraints): Promise<MediaStream>;
  getDisplayMedia(constraints: DisplayMediaStreamOptions): Promise<MediaStream>;
}

export class JsWebRtcEngine implements MediaEngine {
  readonly name = "js-webrtc";

  isAvailable(): boolean {
    return (
      typeof RTCPeerConnection !== "undefined" &&
      typeof navigator !== "undefined" &&
      !!navigator.mediaDevices
    );
  }

  createPeerConnection(configuration: RTCConfiguration): RTCPeerConnection {
    if (typeof RTCPeerConnection === "undefined") {
      throw new Error("WebRTC indisponível");
    }
    return new RTCPeerConnection(configuration);
  }

  getUserMedia(constraints: MediaStreamConstraints): Promise<MediaStream> {
    if (!navigator.mediaDevices?.getUserMedia) {
      return Promise.reject(new Error("captura de microfone indisponível"));
    }
    return navigator.mediaDevices.getUserMedia(constraints);
  }

  getDisplayMedia(constraints: DisplayMediaStreamOptions): Promise<MediaStream> {
    if (!navigator.mediaDevices?.getDisplayMedia) {
      return Promise.reject(new Error("captura de tela indisponível"));
    }
    return navigator.mediaDevices.getDisplayMedia(constraints);
  }
}
