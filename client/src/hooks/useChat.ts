/**
 * Chat state + relay wiring, lifted out of ChatScreen so the screen is pure
 * rendering. Owns: local history load, optimistic send + ack, partner presence
 * + typing, and the typing-start/stop throttle. The socket itself is the app-
 * lifetime singleton from services/ws (owned by App); this hook only subscribes.
 *
 * Optimistic flow: send() inserts the message locally with status "sending",
 * emits `chat` over the socket, and flips to "delivered" when the relay acks.
 *
 * RECEIVE IS NOT HERE: a received message is persisted + routed app-lifetime by
 * the `messages` singleton (services/messages.ts), so it lands in the local DB
 * even while ChatScreen is unmounted (the old bug: a chat arriving on Home was
 * notified but never saved, so opening the chat later showed nothing). This hook
 * now only LOADS history on mount and subscribes to `messages` for live appends
 * + ack flips — the wire handler itself lives in the singleton (reusing the
 * same crypto / DB / notificationQueue utilities).
 */
import { useEffect, useRef, useState } from "react";
import { socket } from "../services/ws";
import { asMs, insertOutgoing, loadMessages, type StoredRow } from "../lib/localDb";
import { loadIdentity } from "../lib/identity";
import { sealTo } from "../lib/crypto";
import { messages as messageStore } from "../services/messages";
import type { Identity, PresenceState, ServerEvent, StoredMessage } from "../lib/types";

function rowToMessage(r: StoredRow): StoredMessage {
  return {
    id: r.id,
    partner_id: r.partner_id,
    text: r.text,
    from_me: r.from_me === 1,
    created_at: asMs(r.created_at),
    status: r.status as StoredMessage["status"],
    image: r.image ?? null,
  };
}

export function useChat(identity: Identity, initialPartnerPresence: PresenceState = "offline") {
  const partnerId = identity.partner_id!;
  const [connected, setConnected] = useState(socket.getStatus());
  const [messages, setMessages] = useState<StoredMessage[]>([]);
  const [draft, setDraft] = useState("");
  const [partnerPresence, setPartnerPresence] = useState<PresenceState>(initialPartnerPresence);
  const [partnerTyping, setPartnerTyping] = useState(false);
  const lastTypingSent = useRef(0);
  const stopTimer = useRef<number | null>(null);
  const partnerTypingTimer = useRef<number | null>(null);

  // Crypto material lives in a ref so `send()` reads the LATEST keys without
  // re-subscribing every time the identity changes. Seeded from the `identity`
  // prop, then reconciled against the store on mount (App.tsx's cold-start
  // upgrade may have just published a key / fetched the partner's, one render
  // ahead of this prop). Reads fresh via `loadIdentity` to avoid closing over a
  // stale `identity`, mirroring the `unpairRef` pattern in App.tsx. The receive
  // side owns its OWN copy of these keys (in the messages singleton); we keep
  // the singleton's partner_pubkey in sync below after the REST refresh so a
  // re-key discovered here still decrypts there.
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
    (async () => {
      const fresh = await loadIdentity();
      if (cancelled || !fresh) return;
      keysRef.current = {
        device_privkey: fresh.device_privkey ?? null,
        device_pubkey: fresh.device_pubkey ?? null,
        partner_pubkey: fresh.partner_pubkey ?? null,
      };
    })();
    const offStatus = socket.onStatus(setConnected);
    // Received chat/ack is processed app-lifetime by the `messages` singleton
    // (so a message arriving while ChatScreen is unmounted is still saved to
    // the DB). Here we only subscribe to its live appends + ack flips, plus the
    // screen-local presence/typing/last_seen that drive header UI.
    const offMessage = messageStore.onMessage((m) => setMessages((prev) => [...prev, m]));
    const offAck = messageStore.onAck(
      (id) => setMessages((prev) => prev.map((x) => (x.id === id ? { ...x, status: "delivered" } : x))),
    );
    const offEvent = socket.onEvent((e: ServerEvent) => {
      // Only the chat-screen-local UI bits; chat/ack are owned by the singleton.
      if (e.type === "presence") setPartnerPresence(e.state);
      else if (e.type === "last_seen") setPartnerPresence(e.presence);
      else if (e.type === "typing") {
        setPartnerTyping(e.state === "start");
        if (e.state === "start") {
          if (partnerTypingTimer.current) clearTimeout(partnerTypingTimer.current);
          partnerTypingTimer.current = window.setTimeout(() => setPartnerTyping(false), 4000);
        }
      }
    });
    return () => {
      cancelled = true;
      offStatus();
      offMessage();
      offAck();
      offEvent();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [partnerId]);

  useEffect(() => {
    setPartnerPresence(initialPartnerPresence);
  }, [initialPartnerPresence]);

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
