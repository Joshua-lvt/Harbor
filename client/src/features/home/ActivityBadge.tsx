/**
 * Activity badges — two flavors of the same exe→label mapping.
 *
 *  - `ActivityBadge`        : the PARTNER's activity (inbound `activity` event).
 *  - `MyActivityBadge`      : my OWN current foreground app (local preview) plus
 *    a sharing status. When sharing is off, shows "Atividade oculta" so it's
 *    obvious nothing is being broadcast.
 *
 * Both map a raw lowercased exe to a friendly name + game via `lib/appNames.ts`.
 * Privacy: only the process name ever crosses the wire — never a window title
 * or screen content (see `useActivity.ts` / `foreground.rs`).
 *
 * Feature 4 — `MyActivityBadge` renders a 20px app icon before the label. The
 * sender already extracted + cached its OWN foreground icons (see
 * `maybeSendActivityIcon` → `getAppIcon(path, exe)`), so a cache-only `useAppIcon`
 * lookup resolves them from memory; a not-yet-extracted exe renders the
 * `GeneratedAppIcon` fallback until the next poll warms the cache.
 */
import { detectGame, friendlyName } from "../../lib/appNames";
import { useAppIcon, GeneratedAppIcon } from "../../lib/appIconCache";

export function ActivityBadge({ exe }: { exe: string | null }) {
  const game = detectGame(exe);
  const label = game ? `Jogando ${game}` : exe ? `Usando ${friendlyName(exe)}` : "Nenhuma atividade";
  const icon = game ? "🎮" : exe ? "💻" : "📭";

  return (
    <div className="text-sm text-harbor-ink/60">
      <span className="inline-flex items-center gap-1.5">
        {icon} {label}
      </span>
    </div>
  );
}

/** A 20px app icon (real if cached, generated fallback otherwise) for the local
 *  activity row. `null` exe → a placeholder circle so the row doesn't shift. */
function MyActivityIcon({ exe }: { exe: string | null }) {
  const icon = useAppIcon(exe ?? "");
  if (!exe) {
    return (
      <span
        className="inline-block h-5 w-5 shrink-0 rounded-md bg-white/15"
        aria-hidden
      />
    );
  }
  return icon ? (
    <img
      src={icon}
      alt=""
      className="inline-block h-5 w-5 shrink-0 rounded object-contain"
      draggable={false}
    />
  ) : (
    <GeneratedAppIcon exe={exe} size={20} className="inline-block shrink-0" />
  );
}

/** My own activity + share status, shown on Home. Compact (header pill). */
export function MyActivityBadge({ exe, sharing }: { exe: string | null; sharing: boolean }) {
  if (!sharing) {
    return (
      <div className="text-xs text-harbor-ink/40">
        <span className="inline-flex items-center gap-1.5">🔒 Atividade oculta</span>
      </div>
    );
  }
  const game = detectGame(exe);
  const label = game ? `Jogando ${game}` : exe ? friendlyName(exe) : "Sem atividade";

  return (
    <div className="text-xs text-harbor-ink/60 min-w-0">
      <span className="inline-flex items-center gap-1.5 truncate">
        <span className="shrink-0">Você:</span>
        <MyActivityIcon exe={exe} />
        <span className="truncate">{label}</span>
      </span>
    </div>
  );
}

export default ActivityBadge;
