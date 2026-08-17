/**
 * Passive pairing detection — the "code creator" side.
 *
 * While PairingScreen is open and this device already has a device_id +
 * device_secret (it has registered and is sitting on its own pairing code),
 * poll GET /me every few seconds. The partner may paste OUR code from their
 * device at any moment; when they do, /me reports a non-null partner_id. On
 * detection we fetch the partner's full info via the existing GET /partner
 * path, persist the partner link (id, name, X25519 public key, avatar) with
 * saveIdentity, and hand the completed identity to `onPaired` — the SAME
 * callback the active /pair path uses — so App.tsx wires the socket,
 * notifications, voice, and navigates to Home exactly as a manual pairing
 * would.
 *
 * No new WebSocket event, no protocol change, no /pair change. The new
 * behavior is strictly: "if someone paired with me while I waited, find out
 * automatically."
 *
 * Reuses existing functions: getMe + getPartner (lib/relay), saveIdentity
 * (lib/identity), and the App.tsx onPaired plumbing (setId + socket.connect +
 * attachNotifications + voice.startAlwaysOn + #/home). The field mapping is
 * shared with the active path via `withPartner` below (no duplicated mapping).
 *
 * Concurrency: a shared `pairingRef` mutex prevents the passive poll from
 * racing an active /pair the user may trigger the same instant (paste +
 * "Conectar"). Whichever acquires pairingRef first wins; the other no-ops.
 *
 * Error handling: transient relay/network errors during polling are swallowed
 * (no per-failure status, no ejection from the screen). If completing the
 * pairing fails (getPartner / saveIdentity error), the lock is released so the
 * manual path or a later tick can still succeed.
 *
 * Polling stops the moment detection completes (onPaired → App routes to Home
 * → PairingScreen unmounts → this hook's cleanup clears the timer), and on
 * unmount regardless.
 */
import { useEffect, useRef } from "react";
import type { MutableRefObject } from "react";
import { getMe, getPartner } from "../lib/relay";
import { loadIdentity, saveIdentity } from "../lib/identity";
import type { Identity } from "../lib/types";

/** Polling interval — every 2.5s. Deliberately not tight: the partner pasting
 *  our code and us noticing a few seconds later is an invisible delay. */
const POLL_MS = 2500;

/** Apply a partner link to an identity: partner id + their published name,
 *  X25519 public key, and avatar (the `?? null` guards preserve the nullable
 *  contract). Pure value transform, shared by the active /pair path
 *  (PairingScreen.handlePair) and the passive-detection path (this hook), so
 *  the field mapping lives in one place. Both `pair()` and `getPartner()`
 *  return these three fields with identical names. */
export function withPartner(
  id: Identity,
  partnerId: string,
  p: { partner_name: string | null; partner_public_key: string | null; partner_avatar: string | null },
): Identity {
  return {
    ...id,
    partner_id: partnerId,
    partner_name: p.partner_name,
    partner_pubkey: p.partner_public_key ?? null,
    partner_avatar: p.partner_avatar ?? null,
  };
}

export function usePassivePairing(opts: {
  /** Credentialed identity = the identity prop with the LIVE device_secret
   *  threaded in (the prop may lag the just-registered secret — register
   *  persists to the store + local `secret` state but does not re-raise to the
   *  parent — PairingScreen builds this exactly like handlePair). Polling is
   *  gated on a non-empty device_secret: /me 401s without it. */
  credentialed: Identity;
  /** Mutex shared with the active paste-pair path; acquire before completing,
   *  release on failure. Held stable across renders (a React ref object). */
  pairingRef: MutableRefObject<boolean>;
  /** Callback that completes the pairing identically to the active /pair path
   *  (App.tsx: setId + socket.connect + notifs + voice + #/home). */
  onPaired: (next: Identity) => void;
}): void {
  const { pairingRef } = opts;
  // Keep the latest identity + callback in refs so the long-lived timer reads
  // fresh values without re-subscribing on every render (onPaired is an inline
  // closure in App that captures live settings).
  const credRef = useRef(opts.credentialed);
  credRef.current = opts.credentialed;
  const onPairedRef = useRef(opts.onPaired);
  onPairedRef.current = opts.onPaired;

  // Gate: only poll once we can authenticate (/me needs device_secret).
  const hasSecret = !!opts.credentialed.device_secret;

  useEffect(() => {
    if (!hasSecret) return;
    let stopped = false;

    const tick = async () => {
      if (stopped || pairingRef.current) return; // a pairing is mid-flight — don't race
      const cred = credRef.current;
      let me: { partner_id: string | null };
      try {
        me = await getMe(cred);
      } catch {
        // Transient network/relay error: swallow. Don't surface a per-failure
        // status and don't leave the pairing screen — the next tick retries.
        return;
      }
      if (stopped || pairingRef.current) return; // active path may have won while we awaited
      if (!me.partner_id) return; // still waiting

      pairingRef.current = true; // acquire — synchronous, before the next await
      let partner;
      try {
        partner = await getPartner(cred);
      } catch {
        // fetch failed: release so the active path or a later tick can still
        // succeed. Don't strand the user on a phantom lock.
        pairingRef.current = false;
        return;
      }
      if (stopped) return; // unmounted mid-fetch
      // Read the persisted identity fresh from the store — NOT credRef.current.
      // The credentialed identity (prop + live secret) never received the
      // X25519 keypair that /register persisted (register stores it but doesn't
      // re-raise it to the parent), so building `next` from the prop would OMIT
      // device_pubkey/device_privkey, and the integral saveIdentity write would
      // wipe the keys the registration just stored. Loading fresh guarantees the
      // keypair travels into `next`, so E2E is live immediately after the passive
      // pairing is detected (no cold-start upgrade needed).
      let stored: Identity | null = null;
      try {
        stored = await loadIdentity();
      } catch {
        // local store read failed — keep the cached credentialed identity
        // (which may or may not carry keys); behavior matches pre-fix here.
      }
      const next = withPartner(stored ?? credRef.current, me.partner_id, partner);
      // Retire the single-use pairing code locally — the relay already nulled
      // both codes on pair (its only signal we were linked came from the
      // partner's /pair). The active /pair path leaves the code as-is and is
      // untouched here on purpose; passive detection retires it because, for
      // the passive side, "now paired" is purely a server-side fact. Null is
      // harmless: pairing_code is unread while paired and reissued (via the
      // `unpaired` WS event, or /me) the moment we unlink, so we never strand
      // the user code-less.
      const retired: Identity = { ...next, pairing_code: null };
      try {
        await saveIdentity(retired);
      } catch {
        pairingRef.current = false; // persist failed: release, don't strand
        return;
      }
      if (!stopped) onPairedRef.current(retired);
    };

    const h = window.setInterval(tick, POLL_MS);
    return () => {
      stopped = true;
      clearInterval(h);
    };
    // Only the credential-gate boolean + the stable ref object are deps; the
    // latest identity/callback reach the timer through refs.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [hasSecret, pairingRef]);
}
