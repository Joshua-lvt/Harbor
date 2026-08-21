/**
 * CallStrip — the always-on voice surface pinned to the TOP of Home's main
 * column (above the PartnerHeroCard). It never takes the column over: the hero
 * + 2×2 info grid stay visible below it. This is the "call integrada à Home",
 * not a separate screen.
 *
 * Layout: two small avatar orbs side by side (local on the left, partner on the
 * right) separated by a connector — a blue animated ring lights around whoever's
 * mic is hot (local while holding Alt = `pttActive`, partner when their remote
 * audio exceeds the speech threshold = `partnerSpeaking`). When neither is
 * talking both wear the faint "idle" ring so the two orbs read as a pair. A
 * one-line status sits to the right of the orbs.
 *
 * State is read straight from the `voice` singleton via `useVoice`. The only
 * user action is the "Permitir microfone" / "Tentar novamente" button
 * (`grantAndConnect`) — there is no entr/sair. `needs_permission` is the first-
 * run gesture gate; once the mic is granted, WebView2 keeps it and the strip
 * shows "Em call · Alt para falar" on every later boot with no button. If the
 * click STILL denies (Windows mic privacy off / WebView2 silent deny), the
 * singleton flips to `mic_blocked` and this strip shows an extra "Abrir config"
 * button that opens `ms-settings:privacy-microphone` via the
 * `open_microphone_settings` Rust command — the only lever left then.
 *
 * The avatars + presence come in as props from HomeScreen (it already tracks
 * partner presence + identity there); voice state is local (its own useVoice
 * subscription — cheap, the singleton is the single source of truth, and
 * VoiceStatusCard already subscribes the same way).
 */
import { Mic, MicOff, PhoneCall, Volume2, AlertTriangle, RefreshCw, Settings as SettingsIcon, MonitorUp, MonitorX } from "lucide-react";
import { useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Avatar } from "../../components/Avatar";
import { useVoice } from "../../hooks/useVoice";
import { platformName } from "../../lib/platform";
import type { Identity, PresenceState } from "../../lib/types";

export function CallStrip({
  identity,
  presence,
}: {
  identity: Identity;
  presence: PresenceState;
}) {
  const {
    status,
    pttActive,
    partnerSpeaking,
    screenSharing,
    partnerScreenSharing,
    error,
    grantAndConnect,
    startScreenShare,
    stopScreenShare,
    attachVideoElement,
  } = useVoice();
  // Remote <video> element for the partner's screen share. Rendered only while
  // `partnerScreenSharing`; attached to the voice singleton so the stream flows
  // into it (and detaches on unmount).
  const videoRef = useRef<HTMLVideoElement | null>(null);
  useEffect(() => {
    if (!videoRef.current) return;
    return attachVideoElement(videoRef.current);
  }, [attachVideoElement]);

  const connected = status === "connected";
  const myName = identity.my_name?.trim() || "Você";
  const partnerName = identity.partner_name?.trim() || "Seu parceiro";
  const partnerDot =
    presence === "online" ? "dot-online" : presence === "away" ? "dot-away" : "dot-offline";

  let statusText = "Em call · Alt para falar";
  let icon = <Volume2 className="h-4 w-4" />;
  let chip = "bg-emerald-100 text-emerald-700 dark:bg-emerald-900/40 dark:text-emerald-300";
  let actionable = false; // show a grant/retry button?
  // mic_blocked shows an EXTRA "open Windows mic settings" button next to the
  // retry, because the only lever left is the OS privacy toggle (see voice.ts).
  let needsSettingsAction = false;

  if (status === "needs_permission") {
    statusText = "Permitir microfone para entrar em call";
    icon = <Mic className="h-4 w-4" />;
    chip = "bg-harbor-sky/40 text-harbor-deep";
    actionable = true;
  } else if (status === "mic_blocked") {
    statusText = `Microfone bloqueado pelo ${platformName()}`;
    icon = <MicOff className="h-4 w-4" />;
    chip = "bg-red-100 text-red-700 dark:bg-red-900/40 dark:text-red-300";
    actionable = true; // "Tentar novamente" re-attempts grantAndConnect
    needsSettingsAction = true; // "Abrir config" opens the OS mic settings
  } else if (status === "connecting") {
    statusText = "Conectando…";
    icon = <PhoneCall className="h-4 w-4" />;
    chip = "bg-amber-100 text-amber-700 dark:bg-amber-900/40 dark:text-amber-300";
  } else if (status === "connected" && pttActive) {
    statusText = "Falando…";
    icon = <Volume2 className="h-4 w-4 ptt-pulse" />;
    chip = "bg-emerald-100 text-emerald-700 dark:bg-emerald-900/40 dark:text-emerald-300";
  } else if (status === "connected") {
    statusText = "Em call · Alt para falar";
    icon = <Mic className="h-4 w-4" />;
    chip = "bg-emerald-100 text-emerald-700 dark:bg-emerald-900/40 dark:text-emerald-300";
  } else if (status === "reconnecting") {
    statusText = "Reconectando…";
    icon = <RefreshCw className="h-4 w-4 animate-spin" />;
    chip = "bg-harbor-sky/40 text-harbor-deep";
  } else if (status === "failed") {
    statusText = error || "A conexão de voz falhou";
    icon = <AlertTriangle className="h-4 w-4" />;
    chip = "bg-red-100 text-red-700 dark:bg-red-900/40 dark:text-red-300";
    actionable = true;
  }

  const buttonLabel =
    status === "failed" || status === "mic_blocked" ? "Tentar novamente" : "Permitir microfone";

  return (
    <>
    <div className="flex items-center gap-4 rounded-2xl border border-harbor-card-border bg-harbor-surface/70 px-4 py-3 shadow-sm backdrop-blur transition">
      {/* The two orbs, centered as a pair. 56px avatars each in a vertical
          stack with name + a per-avatar speaking/idle status label — the bigger
          orbs read as a call pair, with a faint speaking ring brighter in dark
          mode (see style.css `.speaking-ring-lg`). */}
      <div className="flex items-end justify-center gap-3">
        <div className={`flex flex-col items-center gap-1 ${pttActive ? "speaking-pulse" : ""}`}>
          <Avatar
            src={identity.my_avatar ?? null}
            alt={myName}
            size={56}
            ring={pttActive ? "speaking" : "idle"}
          />
          <p className="max-w-[6rem] truncate text-xs font-medium text-harbor-ink">{myName}</p>
          <p className="text-[10px] leading-tight text-emerald-600 dark:text-emerald-300 min-h-[12px]">
            {pttActive ? "Microfone ativo" : ""}
          </p>
        </div>
        <span className="text-harbor-deep/40 self-center pb-4" aria-hidden>
          ⟷
        </span>
        <div className={`flex flex-col items-center gap-1 ${partnerSpeaking ? "speaking-pulse" : ""}`}>
          <Avatar
            src={identity.partner_avatar ?? null}
            alt={partnerName}
            size={56}
            ring={partnerSpeaking ? "speaking" : "idle"}
          />
          <p className="max-w-[6rem] truncate text-xs font-medium text-harbor-ink">{partnerName}</p>
          <p className="text-[10px] leading-tight text-emerald-600 dark:text-emerald-300 min-h-[12px]">
            {partnerSpeaking ? "Falando" : connected ? "Em chamada" : ""}
          </p>
        </div>
      </div>

      {/* Status line (names + state) */}
      <div className="min-w-0 flex-1">
        <p className="truncate text-sm font-semibold text-harbor-ink">
          <span>{myName}</span>
          <span className="mx-1.5 text-harbor-sec">·</span>
          <span className="inline-flex items-center gap-1.5">
            <span className={`inline-block h-2 w-2 rounded-full ${partnerDot}`} />
            <span>{partnerName}</span>
          </span>
        </p>
        <div className="mt-1 flex items-center gap-2">
          <span className={`inline-flex items-center gap-1.5 rounded-full px-2.5 py-0.5 text-xs font-medium ${chip}`}>
            {icon}
            <span className="truncate max-w-[16rem]">{statusText}</span>
          </span>
        </div>
      </div>

      {/* One-time gesture / retry / open-Windows-settings — only when the
          state needs it. `mic_blocked` shows an extra "Abrir config" button
          next to the retry, because the OS privacy toggle is the only lever
          left when the click still denies (see voice.ts / open_microphone_settings). */}
      {actionable && (
        <div className="flex items-center gap-2">
          {needsSettingsAction && (
            <button
              onClick={() => void invoke("open_microphone_settings").catch(() => {})}
              className="inline-flex items-center gap-2 rounded-xl border border-harbor-card-border bg-harbor-surface-strong px-3 py-2 text-sm font-medium text-harbor-ink transition hover:bg-harbor-surface"
            >
              <SettingsIcon className="h-4 w-4" />
              Abrir config
            </button>
          )}
          <button
            onClick={() => void grantAndConnect()}
            className="inline-flex items-center gap-2 rounded-xl bg-harbor-deep px-4 py-2 text-sm font-medium text-white transition hover:bg-harbor-sea"
          >
            <MicOff className="h-4 w-4" />
            {buttonLabel}
          </button>
        </div>
      )}

      {/* Screen share — only while connected. Toggle: start (OS picker) or stop. */}
      {connected && (
        <button
          onClick={() => (screenSharing ? stopScreenShare() : void startScreenShare())}
          title={screenSharing ? "Parar de compartilhar a tela" : "Compartilhar a tela"}
          className={`inline-flex items-center gap-2 rounded-xl border px-3 py-2 text-sm font-medium transition ${
            screenSharing
              ? "border-harbor-sea/50 bg-harbor-sea/15 text-harbor-deep"
              : "border-harbor-card-border bg-harbor-surface-strong text-harbor-ink hover:bg-harbor-surface"
          }`}
        >
          {screenSharing ? <MonitorX className="h-4 w-4" /> : <MonitorUp className="h-4 w-4" />}
          {screenSharing ? "Parar tela" : "Compartilhar tela"}
        </button>
      )}
    </div>

    {/* Partner's screen share — a video panel below the strip. */}
    {partnerScreenSharing && (
      <div className="mt-2 overflow-hidden rounded-2xl border border-harbor-card-border bg-black">
        <video ref={videoRef} autoPlay playsInline muted className="max-h-64 w-full object-contain" />
        <p className="bg-black/80 px-3 py-1 text-center text-xs text-white/80">
          {partnerName} está compartilhando a tela
        </p>
      </div>
    )}
    </>
  );
}

export default CallStrip;
