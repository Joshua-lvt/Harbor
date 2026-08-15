/**
 * ActionBar — Home's right-aligned quick action (NOT a full-width bar).
 *
 * The voice call is always on now (driven by the `voice` singleton, surfaced by
 * the `CallStrip` at the top of the column), so there's no "Entrar em call" /
 * "Sair da chamada" here anymore. The single remaining action is "Abrir chat" →
 * routes to the existing Chat screen.
 *
 * `connected` is retained on the signature but currently unused — kept so Home
 * can pass the same socket-status prop it already tracks (a future
 * chat-disabled-when-offline affordance would use it). The chat target hardcodes
 * #/chat because ActionBar lives only on Home and a Home ↔ Chat hop is the one
 * navigation it owns.
 */
import { MessageCircle } from "lucide-react";

export function ActionBar({
  connected: _connected,
}: {
  /** Socket open? Currently unused; retained for a future offline-aware affordance. */
  connected: boolean;
}) {
  return (
    <div className="flex justify-end gap-3">
      <button
        onClick={() => (window.location.hash = "#/chat")}
        className="inline-flex items-center gap-2 rounded-xl bg-harbor-deep px-4 py-2.5 text-sm font-medium text-white transition hover:bg-harbor-sea"
      >
        <MessageCircle className="h-4 w-4" />
        Abrir chat
      </button>
    </div>
  );
}

export default ActionBar;
