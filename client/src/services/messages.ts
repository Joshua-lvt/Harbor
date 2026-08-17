/**
 * App-lifetime chat receiver + persistence — the counterpart to `voice.ts`.
 *
 * WHY THIS EXISTS: the relay delivers a `chat` event to every socket subscriber.
 * `services/notify.ts` is app-lifetime and fires the OS toast for every one
 * ("Nova mensagem recebida"), but the code that PERSISTS a received message
 * (`insertIncoming`) and renders a bubble used to live only in `useChat` — which
 * is mounted only inside `ChatScreen`. So a message that arrived while the
 * recipient was on Home/Settings was notified but never saved: opening the chat
 * later showed nothing. This singleton processes `chat`/`ack` for the whole app
 * lifetime, persisting received messages regardless of which screen is mounted,
 * exactly like `voice.ts` keeps the call up across route changes.
 *
 * Pattern mirrors `voice.ts`: module-scope singleton, `start/stop` on
 * pair/unpair (idempotent via `running`), subscribes to `socket.onEvent` and
 * emits to listeners. `useChat` becomes a thin consumer — it loads history from
 * the DB on mount and subscribes here for live appends; `send()` stays there.
 *
 * The receive logic (E2E decrypt + harbor_notify routing + plaintext fallback) is
 * a 1:1 port of the old `useChat.handleEvent`, reusing the same crypto / DB /
 * notificationQueue utilities untouched. Private keys never leave the client; the
 * relay is key-blind.
 */
import { socket } from "./ws";
import { openFrom } from "../lib/crypto";
import { insertIncoming, markDelivered } from "../lib/localDb";
import { notificationQueue } from "./notificationQueue";
import { notify } from "./notify";
import { loadSettings } from "../lib/identity";
import type { Identity, ServerEvent, StoredMessage } from "../lib/types";

/** Tag prepended to a keyless quick-message so the receiver still routes it to
 *  the notification queue (mirrors useChat). */
const PLAINTEXT_NOTIF_TAG = "harbor_notify:";

// Placeholder bubbles for the receive-side decrypt-failure paths. An encrypted
// message must NEVER be silently dropped — the user sees something arrived that
// we couldn't read. Stored plaintext in the local DB so a reload shows the same
// placeholder (we never persist the `enc` ciphertext). Mirrors useChat.
const DECRYPT_FAIL = "🔒 Não foi possível descriptografar esta mensagem.";
const DECRYPT_CORRUPT = "🔒 Mensagem corrompida.";
const DECRYPT_NO_KEY = "🔒 Mensagem criptografada — chave ausente.";

type MessageListener = (m: StoredMessage) => void;
type AckListener = (id: string) => void;

class MessageStore {
  private running = false;
  /** Unsubscribe handle for the socket.onEvent subscription owned here. */
  private offEvent: (() => void) | null = null;

  /** Partner id used as the `partner_id` FK for insertIncoming. Null = stopped. */
  private partnerId: string | null = null;
  /** Partner display name + avatar, for the harbor_notify notification cards. */
  private partnerName = "Seu parceiro";
  private partnerAvatar: string | null = null;
  /** Our own keypair, refreshed by App via setPartnerIdentity whenever keys
   *  change (pair / re-key). Only OUR keys are needed to RECEIVE —
   *  `openFrom(priv, pub, enc)` decrypts a sealed box with the recipient's
   *  keypair; the partner's public key is used only by the SENDER (sealTo, in
   *  useChat.send), not here. Reads the freshest values without re-subscribing
   *  (same pattern as useChat.keysRef before the refactor). */
  private devicePrivkey: string | null = null;
  private devicePubkey: string | null = null;

  private messageListeners = new Set<MessageListener>();
  private ackListeners = new Set<AckListener>();

  /** Engage the receiver for a paired session. Idempotent — re-starting for a new
   *  partner first stops the prior session so refs + listeners don't bleed.
   *  Call exactly once when paired (boot + onPaired); pair with `stop()` on
   *  unpair. `ident` seeds the crypto + notification fields; App refreshes them
   *  later via setPartnerIdentity. */
  start(partnerId: string, ident: Identity): void {
    if (this.running && this.partnerId === partnerId) return;
    if (this.running) this.stop();
    this.partnerId = partnerId;
    this.partnerName = ident.partner_name?.trim() || "Seu parceiro";
    this.partnerAvatar = ident.partner_avatar ?? null;
    this.devicePrivkey = ident.device_privkey ?? null;
    this.devicePubkey = ident.device_pubkey ?? null;
    this.running = true;
    if (this.offEvent) this.offEvent();
    this.offEvent = socket.onEvent((e) => void this.handleEvent(e));
  }

  /** Tear down the receiver (called on unpair). Stops processing + drops the
   *  subscription so a stopped singleton doesn't persist against a dead link.
   *  Listeners are kept (harmless — they no-op without emissions). */
  stop(): void {
    this.running = false;
    if (this.offEvent) {
      this.offEvent();
      this.offEvent = null;
    }
    this.partnerId = null;
    this.partnerName = "Seu parceiro";
    this.partnerAvatar = null;
    this.devicePrivkey = null;
    this.devicePubkey = null;
  }

  /** Refresh crypto + notification fields when our keys or the partner's
   *  name/avatar change (re-key, profile_update). Our keypair needs to stay
   *  current to keep decrypting after a re-key; the partner's public key is
   *  NOT taken here (only the sender uses it — see useChat.send). */
  setPartnerIdentity(patch: {
    partner_name?: string | null;
    partner_avatar?: string | null;
    device_privkey?: string | null;
    device_pubkey?: string | null;
  }): void {
    if (!this.running) return;
    if (patch.partner_name !== undefined)
      this.partnerName = patch.partner_name?.trim() || "Seu parceiro";
    if (patch.partner_avatar !== undefined) this.partnerAvatar = patch.partner_avatar ?? null;
    if (patch.device_privkey !== undefined) this.devicePrivkey = patch.device_privkey ?? null;
    if (patch.device_pubkey !== undefined) this.devicePubkey = patch.device_pubkey ?? null;
  }

  /** Live-append a received message. Runs the listener once for the current
   *  set on subscribe? No — appends only; history load stays in useChat. */
  onMessage(l: MessageListener): () => void {
    this.messageListeners.add(l);
    return () => this.messageListeners.delete(l);
  }

  /** Notifies when an outgoing message is acked delivered. The listener maps by
   *  id to flip its bubble; useChat sets status:"delivered". */
  onAck(l: AckListener): () => void {
    this.ackListeners.add(l);
    return () => this.ackListeners.delete(l);
  }

  // --- handling (1:1 port of the old useChat.handleEvent) ------------------

  private async handleEvent(e: ServerEvent): Promise<void> {
    if (!this.running) return;
    if (e.type === "chat") {
      await this.onChat(e);
    } else if (e.type === "ack") {
      if (e.delivered) {
        await markDelivered(e.id);
        this.ackListeners.forEach((l) => l(e.id));
      }
    }
  }

  private async onChat(e: Extract<ServerEvent, { type: "chat" }>): Promise<void> {
    if (!this.partnerId) return; // stopped / not paired
    // The wire envelope is one of two mutually-exclusive shapes (see types.ts):
    // an opaque `enc` sealed-box string (E2E) or a plaintext `text` (+optional
    // `image`) for scripts / pre-E2E partners / a partner that hasn't published a
    // key yet. The local DB always stores PLAINTEXT — we decrypt on receive and
    // persist the readable text; at-rest encryption of the local DB is deferred.
    let text: string;
    let image: string | null = null;
    if ("enc" in e && e.enc) {
      const priv = this.devicePrivkey;
      const pub = this.devicePubkey;
      if (priv && pub) {
        const plain = await openFrom(priv, pub, e.enc);
        if (plain === null) {
          text = DECRYPT_FAIL;
        } else {
          try {
            const inner = JSON.parse(plain) as {
              text?: unknown;
              image?: unknown;
              kind?: unknown;
              preset_id?: unknown;
            };
            if (inner.kind === "harbor_notify") {
              notificationQueue.enqueue({
                id: e.id,
                partnerName: this.partnerName,
                partnerAvatar: this.partnerAvatar,
                text: typeof inner.text === "string" ? String(inner.text) : "",
                presetId:
                  typeof inner.preset_id === "string" ? inner.preset_id : undefined,
                timestamp: e.ts,
                repeatCount: 1,
              });
              return;
            }
            text = typeof inner.text === "string" ? inner.text : "";
            image = typeof inner.image === "string" && inner.image ? inner.image : null;
          } catch {
            text = DECRYPT_CORRUPT;
          }
        }
      } else {
        text = DECRYPT_NO_KEY;
      }
    } else if ("text" in e) {
      if (e.text.startsWith(PLAINTEXT_NOTIF_TAG)) {
        let inner: { kind?: unknown; text?: unknown; preset_id?: unknown } = {};
        try {
          inner = JSON.parse(e.text.slice(PLAINTEXT_NOTIF_TAG.length));
        } catch {
          inner = {};
        }
        if (inner.kind === "harbor_notify") {
          notificationQueue.enqueue({
            id: e.id,
            partnerName: this.partnerName,
            partnerAvatar: this.partnerAvatar,
            text: typeof inner.text === "string" ? String(inner.text) : "",
            presetId:
              typeof inner.preset_id === "string" ? inner.preset_id : undefined,
            timestamp: e.ts,
            repeatCount: 1,
          });
          return;
        }
      }
      text = e.text;
      image = e.image ?? null;
    } else {
      return;
    }
    await insertIncoming(e.id, this.partnerId, text, e.ts, image);
    // OS "Nova mensagem recebida" toast — fired HERE (post-decrypt) rather than
    // in the raw `chat` subscriber so it never fires for Harbor quick-messages
    // (those `return` above, routed to the notification queue + giant overlay
    // long before this line). Non-blocking: the user's toast arrives on its own.
    void (async () => {
      try {
        const s = await loadSettings();
        await notify("💬 Harbor", "Nova mensagem recebida.", s, "notify_on_message");
      } catch {
        /* store read / permission denied — non-fatal: the message is saved. */
      }
    })();
    this.messageListeners.forEach((l) =>
      l({
        id: e.id,
        partner_id: this.partnerId!,
        text,
        from_me: false,
        created_at: e.ts,
        status: "delivered",
        image,
      }),
    );
  }
}

export const messages = new MessageStore();
