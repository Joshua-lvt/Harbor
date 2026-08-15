/**
 * QuickMessageBar — the SENDING surface for Harbor quick-messages (Feature 5).
 *
 * A row of preset buttons + a custom text field that each fire a
 * *notification-only* quick-message: sealed to the partner's public key and
 * sent down the existing E2E `chat.enc` path (identical to a normal chat send —
 * the relay forwards opaque ciphertext and stays key-blind). The receiver
 * detects `inner.kind === "harbor_notify"` in useChat and routes to the
 * notification queue instead of the chat store — so it never shows as a chat
 * bubble and is never persisted.
 *
 * Spam limits: a minimum of SPAM_MIN_GAP_MS between sends and a sliding window
 * of MAX_PER_MIN per minute. Violations are ignored silently (the field won't
 * have `disabled` semantics that hide the cap from a partner who needs to reach
 * you, but the send is dropped client-side).
 *
 * Plaintext fallback: when the partner's key (or our own) is absent — mostly a
 * never-message before both published — we tag the plaintext `text` with a
 * `harbor_notify:` prefix so the receiver still routes it to the queue.
 */
import { useState } from "react";
import { socket } from "../../services/ws";
import { sealTo } from "../../lib/crypto";
import type { Identity } from "../../lib/types";

const PRESETS: { id: string; emoji: string; text: string }[] = [
  { id: "love", emoji: "💛", text: "Eu te amo" },
  { id: "miss", emoji: "🥺", text: "Sinto saudades" },
  { id: "reply", emoji: "⚠️", text: "Me responde!!!" },
  { id: "look", emoji: "👀", text: "Olha o Harbor" },
  { id: "play", emoji: "🎮", text: "Vem jogar comigo" },
  { id: "pause", emoji: "☕", text: "Faz uma pausa" },
  { id: "night", emoji: "🌙", text: "Boa noite" },
  { id: "morning", emoji: "🌞", text: "Bom dia" },
];

const MAX_CHARS = 120;
const SPAM_MIN_GAP_MS = 3000;
const MAX_PER_MIN = 10;
const WINDOW_MS = 60_000;

/** Tag plaintext quick-messages with so the receiver routes them to the
 *  notification queue even without E2E keys (the pre-key fallback path). */
const PLAINTEXT_TAG = "harbor_notify:";

export function QuickMessageBar({
  identity,
  partnerPubkey,
  myPrivkey,
  myPubkey,
  connected,
}: {
  identity: Identity;
  partnerPubkey: string | null;
  myPrivkey: string | null;
  myPubkey: string | null;
  connected: boolean;
}) {
  const [custom, setCustom] = useState("");
  const [sentAt, setSentAt] = useState<number[]>([]);
  const [justSent, setJustSent] = useState<string | null>(null);

  if (!identity.partner_id) return null;

  /** Sliding-window spam guard. Returns true when the send is allowed. */
  function allowNow(): boolean {
    const now = Date.now();
    // Drop sends within the min gap of the last actual send.
    if (sentAt.length && now - sentAt[sentAt.length - 1] < SPAM_MIN_GAP_MS)
      return false;
    // Slide the 1-min window and cap it.
    const fresh = sentAt.filter((t) => now - t < WINDOW_MS);
    if (fresh.length >= MAX_PER_MIN) return false;
    fresh.push(now);
    setSentAt(fresh);
    return true;
  }

  function flashSent(text: string) {
    setJustSent(text);
    window.setTimeout(() => setJustSent(null), 1400);
  }

  async function sendQuick(text: string, presetId?: string) {
    const trimmed = text.trim();
    if (!trimmed || !connected || !identity.partner_id) return;
    if (!allowNow()) return; // over the spam cap — drop silently
    const id = `${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
    // Seal the inner payload. `kind: "harbor_notify"` is what the receiver keys
    // on (useChat) to route to the notification queue instead of the chat store.
    const inner = JSON.stringify({
      kind: "harbor_notify",
      text: trimmed,
      ...(presetId ? { preset_id: presetId } : {}),
    });
    if (myPrivkey && myPubkey && partnerPubkey) {
      const enc = await sealTo(partnerPubkey, inner);
      socket.send({ type: "chat", id, enc });
    } else {
      // Plaintext fallback: tag the text so a keyless partner still routes it.
      socket.send({ type: "chat", id, text: `${PLAINTEXT_TAG}${inner}` });
    }
    flashSent(trimmed);
  }

  const over = custom.length > MAX_CHARS;
  const remaining = MAX_CHARS - custom.length;

  return (
    <div className="flex flex-col gap-2 rounded-xl bg-harbor-surface/70 p-3">
      {/* Preset buttons */}
      <div className="flex flex-wrap gap-2">
        {PRESETS.map((p) => (
          <button
            key={p.id}
            type="button"
            onClick={() => void sendQuick(p.text, p.id)}
            disabled={!connected}
            title={`${p.emoji} ${p.text}`}
            className="inline-flex items-center gap-1 rounded-lg bg-harbor-surface-strong px-2.5 py-1.5 text-xs font-medium text-harbor-ink transition hover:bg-harbor-sea hover:text-white disabled:opacity-40 disabled:hover:bg-harbor-surface-strong disabled:hover:text-harbor-ink"
          >
            <span aria-hidden>{p.emoji}</span>
            {justSent === p.text ? "Enviado ✓" : p.text}
          </button>
        ))}
      </div>

      {/* Custom text + send */}
      <div className="flex items-center gap-2">
        <input
          value={custom}
          onChange={(e) => setCustom(e.target.value.slice(0, MAX_CHARS))}
          onKeyDown={(e) => {
            if (e.key === "Enter") void sendQuick(custom);
          }}
          placeholder="Escreva algo rápido…"
          maxLength={MAX_CHARS}
          className="min-w-0 flex-1 rounded-lg border border-harbor-sky bg-harbor-surface-strong px-3 py-2 text-sm outline-none focus:border-harbor-sea"
        />
        <button
          type="button"
          onClick={() => {
            void sendQuick(custom);
            setCustom("");
          }}
          disabled={!connected || !custom.trim()}
          className="shrink-0 rounded-lg bg-harbor-deep px-3 py-2 text-sm font-medium text-white transition hover:bg-harbor-sea disabled:opacity-40"
        >
          Enviar
        </button>
      </div>
      {/* Live char counter — turns red over the cap (the input also hard-clamps). */}
      <div className="flex justify-end">
        <span
          className={`text-[10px] ${over ? "text-red-500 dark:text-red-300" : "text-harbor-ink/40"}`}
        >
          {remaining}
        </span>
      </div>
    </div>
  );
}

export default QuickMessageBar;
