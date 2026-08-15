/**
 * Chat state + relay wiring, lifted out of ChatScreen so the screen is pure
 * rendering. Owns: local history load, optimistic send + ack, partner presence
 * + typing, and the typing-start/stop throttle. The socket itself is the app-
 * lifetime singleton from services/ws (owned by App); this hook only subscribes.
 *
 * Optimistic flow: send() inserts the message locally with status "sending",
 * emits `chat` over the socket, and flips to "delivered" when the relay acks.
 * Received messages are persisted as "delivered" immediately.
 */
import { useEffect, useRef, useState } from "react";
import { socket } from "../services/ws";
import {
  insertIncoming,
  insertOutgoing,
  loadMessages,
  markDelivered,
  type StoredRow,
} from "../lib/localDb";
import { getPartner } from "../lib/relay";
import { loadIdentity, saveIdentity } from "../lib/identity";
import { openFrom, sealTo } from "../lib/crypto";
import { notificationQueue } from "../services/notificationQueue";
import type { Identity, PresenceState, ServerEvent, StoredMessage } from "../lib/types";

/** Plaintext-fallback tag prepended to a keyless quick-message so the receiver
 *  still routes it to the notification queue (see QuickMessageBar). */
const PLAINTEXT_NOTIF_TAG = "harbor_notify:";

// Placeholder bubbles for the receive-side decrypt-failure paths. An encrypted
// message must NEVER be silently dropped — the user sees something arrived that
// we couldn't read. These strings are stored plaintext in the local DB so a
// reload shows the same placeholder (we never persist the `enc` ciphertext).
const DECRYPT_FAIL = "🔒 Não foi possível descriptografar esta mensagem.";
const DECRYPT_CORRUPT = "🔒 Mensagem corrompida.";
const DECRYPT_NO_KEY = "🔒 Mensagem criptografada — chave ausente.";

function rowToMessage(r: StoredRow): StoredMessage {
  return {
    id: r.id,
    partner_id: r.partner_id,
    text: r.text,
    from_me: r.from_me === 1,
    created_at: r.created_at,
    status: r.status as StoredMessage["status"],
    image: r.image ?? null,
  };
}

export function useChat(identity: Identity) {
  const partnerId = identity.partner_id!;
  const [connected, setConnected] = useState(socket.getStatus());
  const [messages, setMessages] = useState<StoredMessage[]>([]);
  const [draft, setDraft] = useState("");
  const [partnerPresence, setPartnerPresence] = useState<PresenceState>("offline");
  const [partnerTyping, setPartnerTyping] = useState(false);
  const lastTypingSent = useRef(0);
  const stopTimer = useRef<number | null>(null);
  const partnerTypingTimer = useRef<number | null>(null);

  // Crypto material lives in a ref so the once-registered socket handler (see
  // handleEvent) and `send()` read the LATEST keys without re-subscribing every
  // time the identity changes. Seeded from the `identity` prop, then reconciled
  // against the store on mount (App.tsx's cold-start upgrade may have just
  // published a key / fetched the partner's, one render ahead of this prop).
  // Reads fresh via `loadIdentity` to avoid closing over a stale `identity`,
  // mirroring the `unpairRef` pattern in App.tsx.
  const keysRef = useRef({
    device_privkey: identity.device_privkey ?? null,
    device_pubkey: identity.device_pubkey ?? null,
    partner_pubkey: identity.partner_pubkey ?? null,
  });

  // Load local history, seed partner presence + crypto keys, subscribe to the
  // socket. The partner's public key is refreshed from the relay here because
  // it may have changed while we were offline (they reinstalled / re-keyed).
  useEffect(() => {
    let cancelled = false;
    loadMessages(partnerId).then((rows) => setMessages(rows.map(rowToMessage)));
    socket.send({ type: "last_seen" });
    (async () => {
      const fresh = await loadIdentity();
      if (cancelled || !fresh) return;
      keysRef.current = {
        device_privkey: fresh.device_privkey ?? null,
        device_pubkey: fresh.device_pubkey ?? null,
        partner_pubkey: fresh.partner_pubkey ?? null,
      };
      try {
        const p = await getPartner(fresh);
        if (cancelled) return;
        setPartnerPresence(p.presence as PresenceState);
        const partnerPub = p.partner_public_key ?? null;
        const partnerAvatar = p.partner_avatar ?? null;
        if (partnerPub !== keysRef.current.partner_pubkey) {
          keysRef.current.partner_pubkey = partnerPub;
        }
        // Persist the latest partner key + avatar in one shot, so the next cold
        // start starts with both and Home/Chat render the current photo without
        // re-fetching. (One save, never two halves racing on a stale `fresh`.)
        if (partnerPub !== fresh.partner_pubkey || partnerAvatar !== (fresh.partner_avatar ?? null)) {
          await saveIdentity({ ...fresh, partner_pubkey: partnerPub, partner_avatar: partnerAvatar });
        }
      } catch {
        // Relay unreachable — keep the cached keys + avatar. The chat still
        // works: E2E with cached keys, or plaintext fallback if keys are absent.
      }
    })();
    const offStatus = socket.onStatus(setConnected);
    const offEvent = socket.onEvent((e: ServerEvent) => handleEvent(e));
    return () => {
      cancelled = true;
      offStatus();
      offEvent();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [partnerId]);

  async function handleEvent(e: ServerEvent) {
    if (e.type === "chat") {
      // The wire envelope is one of two mutually-exclusive shapes (see
      // types.ts): an opaque `enc` sealed-box string (E2E) or a plaintext
      // `text` (+optional `image`) for scripts / pre-E2E partners / a partner
      // that hasn't published a key yet. The local DB always stores PLAINTEXT —
      // we decrypt on receive and persist the readable text; at-rest encryption
      // of the local DB is a deferred milestone.
      let text: string;
      let image: string | null = null;
      if ("enc" in e && e.enc) {
        const { device_privkey: priv, device_pubkey: pub } = keysRef.current;
        if (priv && pub) {
          // openFrom never throws — null means tampered / wrong key. A failed
          // decrypt is never silently dropped: show a placeholder bubble.
          const plain = await openFrom(priv, pub, e.enc);
          if (plain === null) {
            text = DECRYPT_FAIL;
          } else {
            // The sealed plaintext is JSON: { text, ...(image?{image}:{}) }.
            try {
              const inner = JSON.parse(plain) as {
                text?: unknown;
                image?: unknown;
                kind?: unknown;
                preset_id?: unknown;
              };
              // Feature 5 — a quick-message rides inside `chat.enc` like any
              // chat payload, but signals itself with `kind: "harbor_notify"`.
              // Route it to the notification queue ONLY (no chat bubble, no
              // local DB row); `useHarborNotifications` surfaces it.
              if (inner.kind === "harbor_notify") {
                notificationQueue.enqueue({
                  id: e.id,
                  partnerName: identity.partner_name ?? "Seu parceiro",
                  partnerAvatar: identity.partner_avatar ?? null,
                  text: typeof inner.text === "string" ? String(inner.text) : "",
                  presetId:
                    typeof inner.preset_id === "string" ? inner.preset_id : undefined,
                  timestamp: e.ts,
                  repeatCount: 1,
                });
                return; // NOT stored, NOT rendered as a chat bubble
              }
              text = typeof inner.text === "string" ? inner.text : "";
              image =
                typeof inner.image === "string" && inner.image ? inner.image : null;
            } catch {
              text = DECRYPT_CORRUPT;
            }
          }
        } else {
          // Keys missing on our side (upgrade pending / not yet published) — the
          // message is unreadable but must still surface as a placeholder.
          text = DECRYPT_NO_KEY;
        }
      } else if ("text" in e) {
        // Plaintext fallback path (legacy / scripts / partner without a key).
        // Feature 5: a keyless quick-message is tagged `harbor_notify:` + the
        // inner JSON (see QuickMessageBar). Route it to the notification queue
        // like the E2E path — no chat bubble, no DB row.
        if (e.text.startsWith(PLAINTEXT_NOTIF_TAG)) {
          let inner: { kind?: unknown; text?: unknown; preset_id?: unknown } = {};
          try {
            inner = JSON.parse(e.text.slice(PLAINTEXT_NOTIF_TAG.length));
          } catch {
            inner = {}; // malformed — still notify with the raw text below
          }
          if (inner.kind === "harbor_notify") {
            notificationQueue.enqueue({
              id: e.id,
              partnerName: identity.partner_name ?? "Seu parceiro",
              partnerAvatar: identity.partner_avatar ?? null,
              text: typeof inner.text === "string" ? String(inner.text) : "",
              presetId:
                typeof inner.preset_id === "string" ? inner.preset_id : undefined,
              timestamp: e.ts,
              repeatCount: 1,
            });
            return; // NOT stored, NOT rendered as a chat bubble
          }
        }
        text = e.text;
        image = e.image ?? null;
      } else {
        // Empty/malformed `enc` (the relay guards against this, but defend in
        // depth) — nothing to render; drop quietly rather than store junk.
        return;
      }
      await insertIncoming(e.id, partnerId, text, e.ts, image);
      setMessages((m) => [
        ...m,
        { id: e.id, partner_id: partnerId, text, from_me: false, created_at: e.ts, status: "delivered", image },
      ]);
    } else if (e.type === "ack") {
      if (e.delivered) {
        await markDelivered(e.id);
        setMessages((m) => m.map((x) => (x.id === e.id ? { ...x, status: "delivered" } : x)));
      }
    } else if (e.type === "presence") {
      setPartnerPresence(e.state);
    } else if (e.type === "typing") {
      setPartnerTyping(e.state === "start");
      if (e.state === "start") {
        if (partnerTypingTimer.current) clearTimeout(partnerTypingTimer.current);
        partnerTypingTimer.current = window.setTimeout(() => setPartnerTyping(false), 4000);
      }
    } else if (e.type === "last_seen") {
      setPartnerPresence(e.presence);
    }
  }

  function sendTyping(state: "start" | "stop") {
    const now = Date.now();
    if (state === "start" && now - lastTypingSent.current < 1500) return; // throttle
    lastTypingSent.current = now;
    socket.send({ type: "typing", state });
  }

  /** Draft change + throttled typing-start + auto typing-stop after 1.5s idle. */
  function onDraftChange(value: string) {
    setDraft(value);
    sendTyping("start");
    if (stopTimer.current) clearTimeout(stopTimer.current);
    stopTimer.current = window.setTimeout(() => sendTyping("stop"), 1500);
  }

  async function send(image?: string | null) {
    const text = draft.trim();
    if ((!text && !image) || connected !== "open") return;
    const id = `${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
    const ts = Date.now();
    // Always store plaintext locally (we authored it; at-rest encryption of the
    // local DB is deferred). The WIRE envelope, however, is sealed to the
    // partner's public key when we have both our own keypair and theirs — the
    // relay then forwards opaque ciphertext and never sees the content. Without
    // both keys we send plaintext (the chosen posture), visibly marked insecure
    // by the header indicator in ChatScreen.
    await insertOutgoing(id, partnerId, text, ts, image ?? null);
    setMessages((m) => [
      ...m,
      { id, partner_id: partnerId, text, from_me: true, created_at: ts, status: "sending", image: image ?? null },
    ]);
    setDraft("");
    if (stopTimer.current) clearTimeout(stopTimer.current);
    socket.send({ type: "typing", state: "stop" });
    const { device_privkey: priv, partner_pubkey: partnerPub } = keysRef.current;
    if (priv && partnerPub) {
      // E2E: seal { text, image? } into one opaque base64 string. The relay
      // forwards `enc` verbatim and is fully key-blind.
      const enc = await sealTo(partnerPub, JSON.stringify({ text, ...(image ? { image } : {}) }));
      socket.send({ type: "chat", id, enc });
    } else {
      // Plaintext fallback: no partner key yet, or our own key isn't published.
      socket.send({ type: "chat", id, text, ...(image ? { image } : {}) });
    }
  }

  return { connected, messages, draft, partnerPresence, partnerTyping, onDraftChange, send };
}
