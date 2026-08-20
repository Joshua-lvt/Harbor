/**
 * Thin React binding to the app-lifetime `voice` singleton (services/voice.ts).
 *
 * Subscribes to status updates and re-renders on change. The singleton owns the
 * RTCPeerConnection + Push-to-Talk poll + the always-on reconnect loop, so a call
 * survives route changes (open Chat mid-call) and auto-reconnects on socket drops.
 *
 * The only user-facing action is `grantAndConnect` (the one-time "Permitir
 * microfone" gesture); there is no start/hang-up — the call is always on while
 * paired.
 */
import { useEffect, useState } from "react";
import { voice, type VoiceState } from "../services/voice";

export function useVoice(): VoiceState & {
  /** One-time "Permitir microfone" gesture (also serves as "Tentar novamente" on
   *  a failed state). Acquires the mic (now that there's a user gesture) and
   *  routes to the role — the offerer offers, the responder waits. */
  grantAndConnect: () => Promise<void>;
} {
  const [state, setState] = useState<VoiceState>(voice.getState());
  useEffect(() => voice.onState(setState), []);
  return { ...state, grantAndConnect: () => voice.grantAndConnect() };
}
