/**
 * InfoStatCard — a reusable tile for Home's 2×2 info grid.
 *
 * Presentational: an icon, a small secondary label, and a prominent value.
 * Frosted-glass white surface, soft #D6E6FF border, gentle shadow that lifts
 * on hover. `accent` optionally tints the icon chip (e.g. emerald for an active
 * call) so a card can signal state without a separate status line.
 *
 * Sibling: VoiceStatusCard wraps this to bind real `useVoice` state.
 */
import type { ReactNode } from "react";

export function InfoStatCard({
  icon,
  label,
  value,
  accent,
}: {
  icon: ReactNode;
  label: string;
  value: string;
  /** Tailwind classes for the icon chip background/text tint (e.g. "bg-emerald-100 text-emerald-700"). */
  accent?: string;
}) {
  return (
    <div className="rounded-2xl border border-harbor-card-border bg-harbor-surface/70 px-4 py-3 shadow-sm backdrop-blur transition duration-200 hover:-translate-y-0.5 hover:shadow-md">
      <div className="flex items-center gap-3">
        <span
          className={`flex h-9 w-9 shrink-0 items-center justify-center rounded-xl ${
            accent ?? "bg-harbor-sky/40 text-harbor-deep"
          }`}
        >
          {icon}
        </span>
        <div className="min-w-0">
          <p className="text-xs font-medium text-harbor-sec">{label}</p>
          <p className="truncate text-sm font-semibold text-harbor-ink">{value}</p>
        </div>
      </div>
    </div>
  );
}

export default InfoStatCard;
