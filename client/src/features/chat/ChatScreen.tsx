/**
 * Chat screen — secondary view reached from Home (`#/chat`).
 *
 * Pure rendering now: all state + relay wiring lives in `useChat`. Shows the
 * partner header (presence dot + animated typing indicator), date-separated
 * message list with pop-in bubbles, and a composer with emoji picker, image
 * attachment (compressed client-side, sent inline as a JPEG data URL), and
 * Enter-to-send / Shift+Enter for newline. `send()` takes an optional `image`
 * data URL — the 📎 picker passes one; the Enviar button and Enter call it bare.
 * Owns `usePresence` (OS-wide idle)
 * while this route is active.
 */
import { useEffect, useRef, useState, type ReactNode } from "react";
import { useChat } from "../../hooks/useChat";
import { usePresence } from "../../hooks/usePresence";
import { loadSettings } from "../../lib/identity";
import { fileToCompressedDataUrl } from "../../lib/image";
import type { Identity, Settings } from "../../lib/types";
import { DEFAULT_SETTINGS } from "../../lib/types";
import { SharkMascot } from "../../assets/shark";
import { Avatar } from "../../components/Avatar";
import ChatBubble from "./ChatBubble";
import DateSeparator from "./DateSeparator";
import EmojiButton from "./EmojiButton";
import TypingIndicator from "./TypingIndicator";

function sameDay(a: number, b: number): boolean {
  const da = new Date(a);
  const db = new Date(b);
  return (
    da.getFullYear() === db.getFullYear() &&
    da.getMonth() === db.getMonth() &&
    da.getDate() === db.getDate()
  );
}

export default function ChatScreen({ identity }: { identity: Identity }) {
  const [settings, setSettings] = useState<Settings>(DEFAULT_SETTINGS);
  const { connected, messages, draft, partnerPresence, partnerTyping, onDraftChange, send } =
    useChat(identity);
  const scroller = useRef<HTMLDivElement>(null);
  const fileInput = useRef<HTMLInputElement>(null);
  const [attachBusy, setAttachBusy] = useState(false);
  const [attachErr, setAttachErr] = useState("");

  async function onPickImage(files: FileList | null) {
    if (!files || !files[0]) return;
    setAttachBusy(true);
    setAttachErr("");
    try {
      const dataUrl = await fileToCompressedDataUrl(files[0]);
      send(dataUrl);
    } catch (e) {
      setAttachErr(e instanceof Error ? e.message : "Falha ao anexar imagem.");
    } finally {
      setAttachBusy(false);
      if (fileInput.current) fileInput.current.value = ""; // allow re-picking the same file
    }
  }

  useEffect(() => {
    loadSettings().then(setSettings);
  }, []);
  usePresence(settings.away_after_minutes, connected === "open");

  // Auto-scroll to the newest message / when the typing indicator appears.
  useEffect(() => {
    const el = scroller.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [messages, partnerTyping]);

  const presenceDot =
    partnerPresence === "online" ? "dot-online" : partnerPresence === "away" ? "dot-away" : "dot-offline";
  const presenceLabel =
    partnerPresence === "online" ? "Online" : partnerPresence === "away" ? "Ausente" : "Offline";

  // E2E security indicator — reflects whether BOTH sides have published keys so
  // messages are actually sealed on the wire. We have our own keypair + the
  // partner's pubkey? 🔒 active. Anything missing (upgrade pending / partner
  // hasn't published yet / a script-only partner with no key)? ⚠ pending, and
  // the client visibly falls back to plaintext — never silent, never blocking.
  const encrypted = !!(identity.device_privkey && identity.partner_pubkey);

  // Insert a DateSeparator whenever the calendar day changes between messages.
  const rows: ReactNode[] = [];
  let lastTs: number | null = null;
  for (const m of messages) {
    if (lastTs === null || !sameDay(lastTs, m.created_at)) {
      rows.push(<DateSeparator key={`d-${m.id}`} timestamp={m.created_at} />);
    }
    rows.push(<ChatBubble key={m.id} message={m} />);
    lastTs = m.created_at;
  }

  return (
    <div className="window-main h-screen flex flex-col">
      {/* Header */}
      <div className="flex items-center gap-3 px-4 py-3 border-b border-harbor-line bg-harbor-surface/60 backdrop-blur">
        <button
          className="text-harbor-deep/60 hover:text-harbor-deep text-sm"
          onClick={() => (window.location.hash = "#/home")}
        >
          ‹ Home
        </button>
        <Avatar src={identity.partner_avatar ?? null} alt={identity.partner_name || "Seu parceiro"} size={36} />
        <div className="flex-1 min-w-0">
          <p className="font-semibold text-harbor-deep truncate">
            {identity.partner_name || "Seu parceiro"}
          </p>
          <div className="flex items-center gap-1.5">
            <span className={`inline-block w-2 h-2 rounded-full ${presenceDot}`} />
            <span className="text-xs text-harbor-ink/70">{presenceLabel}</span>
            <TypingIndicator visible={partnerTyping} />
          </div>
        </div>
        <span
          className="text-xs text-harbor-ink/70 select-none whitespace-nowrap"
          title={encrypted ? "Mensagens criptografadas ponta a ponta" : "Criptografia pendente — enviando em texto plano"}
        >
          {encrypted ? "🔒" : "⚠ Criptografia pendente"}
        </span>
        <button
          className="text-xs text-harbor-deep/70 hover:underline"
          onClick={() => (window.location.hash = "#/settings")}
        >
          ⚙
        </button>
      </div>

      {/* Messages */}
      <div ref={scroller} className="flex-1 overflow-y-auto px-4 py-3 flex flex-col gap-2">
        {messages.length === 0 && (
          <div className="m-auto flex flex-col items-center gap-3 text-harbor-ink/60">
            <SharkMascot className="harbor-mascot w-16 h-16" />
            <p className="text-sm">Diga oi. 💙</p>
          </div>
        )}
        {rows}
      </div>

      {/* Composer */}
      <div className="px-3 py-3 border-t border-harbor-line bg-harbor-surface/60 backdrop-blur">
        {connected !== "open" && (
          <p className="reconnect-pulse text-xs text-center text-harbor-ink/60 mb-1">Reconectando…</p>
        )}
        {attachErr && (
          <p className="text-xs text-red-600 dark:text-red-400 mb-1">{attachErr}</p>
        )}
        <div className="flex gap-2 items-end">
          <EmojiButton onPick={(e) => onDraftChange(draft + e)} />
          <input
            ref={fileInput}
            type="file"
            accept="image/*"
            className="hidden"
            onChange={(e) => void onPickImage(e.target.files)}
          />
          <button
            type="button"
            className="text-lg px-1 text-harbor-deep/70 hover:text-harbor-deep disabled:opacity-40"
            disabled={connected !== "open" || attachBusy}
            title="Anexar imagem"
            aria-label="Anexar imagem"
            onClick={() => fileInput.current?.click()}
          >
            {attachBusy ? "⏳" : "📎"}
          </button>
          <textarea
            value={draft}
            onChange={(e) => onDraftChange(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && !e.shiftKey) {
                e.preventDefault();
                send();
              }
            }}
            placeholder="Mensagem"
            rows={1}
            className="flex-1 resize-none rounded-xl border border-harbor-sky bg-harbor-surface-strong px-3 py-2 text-sm outline-none focus:border-harbor-sea max-h-32"
          />
          <button
            onClick={() => send()}
            disabled={connected !== "open" || !draft.trim()}
            className="rounded-xl bg-harbor-deep text-white px-4 py-2 text-sm font-medium hover:bg-harbor-sea disabled:opacity-50 transition"
          >
            Enviar
          </button>
        </div>
      </div>
    </div>
  );
}
