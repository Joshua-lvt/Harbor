/**
 * NotificationsScreen — the SEND surface for Harbor quick-messages (Feature 5),
 * moved off the Home screen into its own tab so Home stays about presence +
 * activity only.
 *
 * Holds the existing `QuickMessageBar` (presets + custom text), which fires a
 * notification-only E2E quick-message down the `chat.enc` path. On the receiver
 * side the message is routed to the giant always-on-top `notif-overlay` window
 * (see hooks/useHarborNotifications.ts + notif-overlay/NotificationOverlay.tsx) —
 * never the OS notification, never the chat store.
 *
 * Routed at `#/notifications` (App.tsx getRoute()); the Sidebar shows a
 * "Notificações" item. Owns no socket — it reuses the app-lifetime socket
 * status that App already tracks (passed in as `connected`).
 */
import { useEffect, useState } from "react";
import { socket } from "../../services/ws";
import { QuickMessageBar } from "../home/QuickMessageBar";
import type { Identity } from "../../lib/types";

export default function NotificationsScreen({
  identity,
  back,
}: {
  identity: Identity;
  back: () => void;
}) {
  const [connected, setConnected] = useState(socket.getStatus());

  useEffect(() => {
    const off = socket.onStatus(setConnected);
    return () => off();
  }, []);

  if (!identity.partner_id) return null;

  return (
    <div className="window-main h-screen flex flex-col">
      {/* Header — matches the ActivitiesScreen chrome. */}
      <div className="flex items-center gap-3 px-4 py-3 border-b border-harbor-line bg-harbor-surface/60 backdrop-blur">
        <button onClick={back} className="text-sm text-harbor-deep">
          ← Voltar
        </button>
        <span className="font-semibold text-harbor-deep">Notificações</span>
      </div>

      <div className="flex-1 overflow-y-auto p-4">
        <div className="mx-auto flex max-w-xl flex-col gap-4">
          {/* Intro — what this tab is for (quick pokes that surface hugely on
              the partner's screen, not a chat message). */}
          <div className="rounded-2xl bg-harbor-surface/70 p-4">
            <h1 className="text-base font-semibold text-harbor-ink">
              Avisos rápidos
            </h1>
            <p className="mt-1 text-sm text-harbor-ink/60">
              Toque em um aviso para enviá-lo. No parceiro, a mensagem aparece em
              destaque na tela dele — grande, por cima de tudo. Não é uma
              mensagem de chat e não salva no histórico.
            </p>
          </div>

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
  );
}
