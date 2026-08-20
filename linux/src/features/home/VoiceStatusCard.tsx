/**
 * VoiceStatusCard — the "Canal de voz" tile in Home's info grid. A passive
 * reflection of the same `voice` singleton the `CallStrip` drives from (it
 * subscribes to `useVoice` itself — the singleton is the single source of
 * truth, so this tile and the strip never disagree). The CallStrip is the
 * primary call surface (with the gesture button); this tile is the at-a-glance
 * "are we connected" read in the grid.
 *
 * State mapping (mirrors CallStrip so the two always agree):
 *   needs_permission → "Permitir microfone"        (sky chip)
 *   mic_blocked      → "Microfone bloqueado pelo Windows" (red chip)
 *   connecting        → "Conectando…"                (amber chip)
 *   connected + ptt   → "Falando…"                   (emerald chip, pulsing)
 *   connected          → "Em call · Alt para falar"   (emerald chip)
 *   reconnecting       → "Reconectando…"              (sky chip)
 *   failed             → error || "Sem conexão"       (red chip)
 */
import { Mic, PhoneCall, Volume2, AlertTriangle, RefreshCw } from "lucide-react";
import { useVoice } from "../../hooks/useVoice";
import { InfoStatCard } from "./InfoStatCard";

export function VoiceStatusCard() {
  const { status, pttActive, error } = useVoice();

  if (status === "needs_permission") {
    return (
      <InfoStatCard
        icon={<Mic className="h-5 w-5" />}
        label="Canal de voz"
        value="Permitir microfone"
        accent="bg-harbor-sky/40 text-harbor-deep"
      />
    );
  }
  if (status === "mic_blocked") {
    return (
      <InfoStatCard
        icon={<AlertTriangle className="h-5 w-5" />}
        label="Canal de voz"
        value="Microfone bloqueado pelo Windows"
        accent="bg-red-100 text-red-700 dark:bg-red-900/40 dark:text-red-300"
      />
    );
  }
  if (status === "connecting") {
    return (
      <InfoStatCard
        icon={<PhoneCall className="h-5 w-5" />}
        label="Canal de voz"
        value="Conectando…"
        accent="bg-amber-100 text-amber-700 dark:bg-amber-900/40 dark:text-amber-300"
      />
    );
  }
  if (status === "connected" && pttActive) {
    return (
      <InfoStatCard
        icon={<Volume2 className="h-5 w-5 ptt-pulse" />}
        label="Canal de voz"
        value="Falando…"
        accent="bg-emerald-100 text-emerald-700 dark:bg-emerald-900/40 dark:text-emerald-300"
      />
    );
  }
  if (status === "connected") {
    return (
      <InfoStatCard
        icon={<Mic className="h-5 w-5" />}
        label="Canal de voz"
        value="Em call · Alt para falar"
        accent="bg-emerald-100 text-emerald-700 dark:bg-emerald-900/40 dark:text-emerald-300"
      />
    );
  }
  if (status === "reconnecting") {
    return (
      <InfoStatCard
        icon={<RefreshCw className="h-5 w-5" />}
        label="Canal de voz"
        value="Reconectando…"
        accent="bg-harbor-sky/40 text-harbor-deep"
      />
    );
  }
  return (
    <InfoStatCard
      icon={<AlertTriangle className="h-5 w-5" />}
      label="Canal de voz"
      value={error || "Sem conexão"}
      accent="bg-red-100 text-red-700 dark:bg-red-900/40 dark:text-red-300"
    />
  );
}

export default VoiceStatusCard;
