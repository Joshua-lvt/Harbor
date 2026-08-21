/**
 * NotificationsScreen — the in-app Harbor notification history (Bug 4).
 *
 * The user wanted Harbor's OWN notification surface, not the OS default toasts.
 * This tab renders every received notification (chat message, presence change,
 * quick-message) in a custom Harbor layout, read from the `notificationStore`
 * singleton (persisted to SQLite). The OS toast fires independently (see
 * services/notify.ts) whenever the relevant toggle is on; this list is the
 * persistent in-app history.
 *
 * The quick-message SEND surface (QuickMessageBar) is kept as a secondary
 * section below the list so the "avisos rápidos" feature isn't lost.
 *
 * Routed at `#/notifications` (App.tsx getRoute()); the Sidebar shows a
 * "Notificações" item. Owns no socket — it reuses the app-lifetime socket
 * status that App already tracks (passed in as `connected`).
 */
import { useEffect, useState } from "react";
import { socket } from "../../services/ws";
import { QuickMessageBar } from "../home/QuickMessageBar";
import { notificationStore, type StoredNotification } from "../../services/notificationStore";
import type { Identity } from "../../lib/types";

/** Compact relative time in pt-BR ("agora", "há 5 min", "há 2 h", "ontem"...). */
function relativeTime(ts: number): string {
  const diff = Date.now() - ts;
  if (diff < 60_000) return "agora";
  const min = Math.floor(diff / 60_000);
  if (min < 60) return `há ${min} min`;
  const h = Math.floor(min / 60);
  if (h < 24) return `há ${h} h`;
  const d = Math.floor(h / 24);
  if (d === 1) return "ontem";
  if (d < 7) return `há ${d} dias`;
  return new Date(ts).toLocaleDateString("pt-BR", { day: "2-digit", month: "2-digit" });
}

const KIND_META: Record<StoredNotification["kind"], { label: string; chip: string }> = {
  message: { label: "Mensagem", chip: "bg-harbor-sky/30 text-harbor-deep" },
  presence: { label: "Presença", chip: "bg-emerald-100 text-emerald-700 dark:bg-emerald-900/40 dark:text-emerald-300" },
  quick: { label: "Aviso rápido", chip: "bg-amber-100 text-amber-700 dark:bg-amber-900/40 dark:text-amber-300" },
};

export default function NotificationsScreen({
  identity,
  back,
}: {
  identity: Identity;
  back: () => void;
}) {
  const [connected, setConnected] = useState(socket.getStatus());
  const [items, setItems] = useState<StoredNotification[]>([]);

  useEffect(() => {
    const off = socket.onStatus(setConnected);
    return () => off();
  }, []);

  // Subscribe to the store; the handler runs once immediately with the current
  // list (after the async load resolves).
  useEffect(() => {
    const off = notificationStore.onChange(setItems);
    return off;
  }, []);

  if (!identity.partner_id) return null;

  const unread = items.filter((n) => !n.read).length;

  return (
    <div className="window-main h-screen flex flex-col">
      {/* Header — matches the ActivitiesScreen chrome. */}
      <div className="flex items-center gap-3 px-4 py-3 border-b border-harbor-line bg-harbor-surface/60 backdrop-blur">
        <button onClick={back} className="text-sm text-harbor-deep">
          ← Voltar
        </button>
        <span className="font-semibold text-harbor-deep">Notificações</span>
        {unread > 0 && (
          <span className="ml-auto rounded-full bg-harbor-deep px-2 py-0.5 text-xs font-semibold text-white">
            {unread} nova{unread > 1 ? "s" : ""}
          </span>
        )}
      </div>

      <div className="flex-1 overflow-y-auto p-4">
        <div className="mx-auto flex max-w-xl flex-col gap-4">
          {/* Received-notification history — the Harbor-styled list. */}
          <div className="rounded-2xl bg-harbor-surface/70 p-4">
            <div className="flex items-center justify-between">
              <h1 className="text-base font-semibold text-harbor-ink">Histórico</h1>
              <div className="flex items-center gap-2">
                {items.length > 0 && (
                  <>
                    <button
                      onClick={() => void notificationStore.markAllRead()}
                      className="text-xs font-medium text-harbor-deep hover:underline"
                    >
                      Marcar lidas
                    </button>
                    <button
                      onClick={() => void notificationStore.clear()}
                      className="text-xs font-medium text-harbor-ink/50 hover:underline"
                    >
                      Limpar
                    </button>
                  </>
                )}
              </div>
            </div>

            {items.length === 0 ? (
              <p className="mt-3 text-sm text-harbor-ink/50">
                Nenhuma notificação ainda. Mensagens, mudanças de presença e
                avisos rápidos do seu parceiro aparecem aqui.
              </p>
            ) : (
              <ul className="mt-3 flex flex-col gap-2">
                {items.map((n) => {
                  const meta = KIND_META[n.kind];
                  return (
                    <li key={n.id}>
                      <button
                        onClick={() => void notificationStore.markRead(n.id)}
                        className={`flex w-full items-start gap-3 rounded-xl border px-3 py-2.5 text-left transition ${
                          n.read
                            ? "border-harbor-card-border bg-harbor-surface-strong/60"
                            : "border-harbor-sky/40 bg-harbor-surface-strong"
                        }`}
                      >
                        <span className="mt-0.5 flex h-9 w-9 shrink-0 items-center justify-center rounded-lg bg-harbor-surface text-lg">
                          {n.icon ?? "🔔"}
                        </span>
                        <span className="min-w-0 flex-1">
                          <span className="flex items-center gap-2">
                            <span className="truncate text-sm font-semibold text-harbor-ink">
                              {n.title}
                            </span>
                            <span className={`shrink-0 rounded-full px-2 py-0.5 text-[10px] font-medium ${meta.chip}`}>
                              {meta.label}
                            </span>
                          </span>
                          <span className="mt-0.5 block text-sm text-harbor-ink/70">{n.body}</span>
                          <span className="mt-0.5 block text-[11px] text-harbor-ink/40">
                            {relativeTime(n.timestamp)}
                          </span>
                        </span>
                        {!n.read && (
                          <span className="mt-1.5 h-2 w-2 shrink-0 rounded-full bg-harbor-sea" />
                        )}
                      </button>
                    </li>
                  );
                })}
              </ul>
            )}
          </div>

          {/* Quick-message SEND surface — kept as a secondary section. */}
          <div className="rounded-2xl bg-harbor-surface/70 p-4">
            <h2 className="text-sm font-semibold text-harbor-ink">Enviar aviso rápido</h2>
            <p className="mt-1 mb-3 text-xs text-harbor-ink/60">
              Aparece em destaque na tela do seu parceiro — não é uma mensagem de
              chat e não salva no histórico dele.
            </p>
            <QuickMessageBar
              identity={identity}
              partnerPubkey={identity.partner_pubkey ?? null}
              myPrivkey={identity.device_privkey ?? null}
              myPubkey={identity.device_pubkey ?? null}
              connected={connected === "open"}
            />
          </div>
        </div>
      </div>
    </div>
  );
}
