/**
 * PartnerHeroCard — the large hero strip at the top of Home's main column.
 *
 * Ocean-gradient card (≈ #63B3ED → #2B6CB0), white text, rounded-3xl. The
 * partner shark floats gently inside a glowing circle on the left; the right
 * holds the partner name, a live presence dot + label, and the slogan line.
 * Pure presentation — presence is fed from HomeScreen's live state.
 */
import { Avatar } from "../../components/Avatar";
import type { PresenceState } from "../../lib/types";

export function PartnerHeroCard({
  partnerName,
  partnerAvatar,
  presence,
  presenceLabel,
}: {
  partnerName: string;
  partnerAvatar?: string | null;
  presence: PresenceState;
  presenceLabel: string;
}) {
  const dot =
    presence === "online" ? "dot-online" : presence === "away" ? "dot-away" : "dot-offline";
  // Derive the subline from the real presence instead of the stale hard-coded
  // "Conectado agora" — which read as a lie when the partner was offline/away.
  const subline =
    presence === "online"
      ? "Conectado agora • Estamos juntos, mesmo longe"
      : presence === "away"
        ? "Ausente • Estamos juntos, mesmo longe"
        : "Offline • Estamos juntos, mesmo longe";

  return (
    <div className="relative overflow-hidden rounded-3xl bg-gradient-to-br from-harbor-sea to-harbor-deep p-6 text-white shadow-lg">
      {/* soft inner glow for depth */}
      <div className="pointer-events-none absolute -right-10 -top-10 h-40 w-40 rounded-full bg-white/15 blur-2xl" />

      <div className="relative flex items-center gap-5">
        {/* Partner avatar / mascot in a glowing circle */}
        <div className="relative flex h-20 w-20 shrink-0 items-center justify-center">
          <div className="absolute inset-0 rounded-full bg-white/20 shadow-[0_0_24px_rgba(255,255,255,0.45)]" />
          <Avatar
            src={partnerAvatar ?? null}
            alt={partnerName || "Seu parceiro"}
            size={64}
            className="shark-float"
          />
        </div>

        <div className="min-w-0">
          <h1 className="truncate text-2xl font-bold leading-tight">
            {partnerName || "Seu parceiro"}
          </h1>
          <div className="mt-1.5 flex items-center gap-2 text-sm text-white/90">
            <span className={`inline-block h-2.5 w-2.5 rounded-full ${dot}`} />
            <span>{presenceLabel}</span>
          </div>
          <p className="mt-1 text-sm text-white/80">
            {subline}
          </p>
        </div>
      </div>
    </div>
  );
}

export default PartnerHeroCard;
