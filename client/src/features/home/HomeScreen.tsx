/**
 * Home — the first screen for a paired device. NOT chat. Focused on the
 * partner: presence, call status, current activity, last activity time.
 * "Estamos juntos, mesmo longe."
 *
 * Redesigned as a two-column desktop layout: a dark glassy `Sidebar` (nav +
 * local user + live activity) on the left, and a main column with a gradient
 * `PartnerHeroCard`, a 2×2 info grid (`InfoStatCard`s + `VoiceStatusCard`),
 * and a right-aligned `ActionBar`, all over a subtle `OceanBackground`.
 *
 * Wiring is unchanged from the pre-redesign Home: it still owns `usePresence`
 * (OS-wide idle) while this route is active, seeds the partner's presence via
 * REST + the `last_seen` WS request, subscribes to presence events, and reads
 * partner activity from the app-lifetime `App.tsx` state passed as props.
 * Voice is driven by the always-on `voice` singleton: a `CallStrip` pinned to
 * the top of the column reflects its state (and owns the one "Permitir
 * microfone" gesture); the RTCPeerConnection + reconnect loop live there.
 */
import { useEffect, useState } from "react";
import { Clock, Gamepad2, MessageSquare } from "lucide-react";
import { socket } from "../../services/ws";
import { usePresence } from "../../hooks/usePresence";
import { useTimeTogether, togetherLabel } from "../../hooks/useTimeTogether";
import { loadMessages } from "../../lib/localDb";
import { detectGame, friendlyName } from "../../lib/appNames";
import { useAppIcon, GeneratedAppIcon, getAppIcon } from "../../lib/appIconCache";
import type { Identity, PresenceState, ServerEvent } from "../../lib/types";
import OceanBackground from "./OceanBackground";
import Sidebar from "./Sidebar";
import PartnerHeroCard from "./PartnerHeroCard";
import InfoStatCard from "./InfoStatCard";
import VoiceStatusCard from "./VoiceStatusCard";
import ActionBar from "./ActionBar";
import CallStrip from "./CallStrip";

function agoLabel(secs: number): string {
  if (secs < 60) return "agora mesmo";
  const m = Math.floor(secs / 60);
  if (m < 60) return `há ${m} min`;
  const h = Math.floor(m / 60);
  if (h < 24) return `há ${h} h`;
  const d = Math.floor(h / 24);
  return `há ${d} d`;
}

export default function HomeScreen({
  identity,
  partnerActivity,
  myActivity,
  myActivityPath,
  shareActivity,
  awayAfterMinutes,
  partnerPresence,
}: {
  identity: Identity;
  partnerActivity: string | null;
  myActivity: string | null;
  /** Full image-name path of MY foreground exe — used by the sender to extract
   *  its OWN icon once per exe (Feature 4). The receiver side never needs it. */
  myActivityPath: string | null;
  shareActivity: boolean;
  awayAfterMinutes: number;
  partnerPresence: PresenceState;
}) {
  const partnerId = identity.partner_id!;
  const [presence, setPresence] = useState<PresenceState>(partnerPresence);
  const [connected, setConnected] = useState(socket.getStatus());
  const [now, setNow] = useState(Date.now());
  const [lastMsgAt, setLastMsgAt] = useState<number | null>(null);
  const [hash, setHash] = useState(typeof window !== "undefined" ? window.location.hash : "#/home");

  usePresence(awayAfterMinutes, connected === "open");

  useEffect(() => {
    setPresence(partnerPresence);
  }, [partnerPresence]);

  // Feature 4 — warm MY OWN foreground icon from the full path so the Sidebar's
  // "Você:" badge (`useAppIcon(myActivity)`, cache-only) renders the real icon
  // even when sharing is OFF (the `useActivity` broadcast path only extracts
  // while sharing). Idempotent — `getAppIcon` is a memory→SQLite→extract cache;
  // repeating a resolved key is a memory hit. The hook re-resolves reactively.
  useEffect(() => {
    if (!myActivity) return;
    void getAppIcon(myActivityPath ?? "", myActivity);
  }, [myActivity, myActivityPath]);
  // "Juntos hoje" — cumulative seconds both devices were online together today
  // (persisted per-day in localStorage; see hooks/useTimeTogether.ts). Replaces
  // the old hardcoded "2h 14min" placeholder. Gated on our socket being open AND
  // the partner's presence being online so the clock only runs when we're truly
  // together.
  const togetherSeconds = useTimeTogether(connected === "open", presence);

  // Presence is owned by App and arrives through the app-lifetime socket.
  useEffect(() => {
    const offStatus = socket.onStatus(setConnected);
    const offEvent = socket.onEvent((e: ServerEvent) => {
      if (e.type === "presence") {
        setPresence(e.state);
      } else if (e.type === "last_seen") {
        setPresence(e.presence);
      }
    });
    return () => {
      offStatus();
      offEvent();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Last local message timestamp — for the "Última interação" card. Reloaded on
  // mount (and would refresh on a future socket ack if we subscribed; the
  // plain mount load is enough for the card's coarse "há X min" label).
  useEffect(() => {
    loadMessages(partnerId)
      .then((rows) => {
        const last = rows.length ? rows[rows.length - 1] : null;
        if (!last) {
          setLastMsgAt(null);
          return;
        }
        // Normalize stored mtimes: outgoing rows store Date.now() (ms, ~1.7e12),
        // incoming rows store the server's time.time() (seconds, ~1.7e9). Promote
        // seconds → ms so the comparison below is always in milliseconds.
        const ts = last.created_at;
        setLastMsgAt(ts < 1e12 ? ts * 1000 : ts);
      })
      .catch(() => {});
    const offEvent = socket.onEvent((e: ServerEvent) => {
      if (e.type === "chat") setLastMsgAt(e.ts * 1000); // server seconds → ms
      else if (e.type === "ack" && e.delivered) setLastMsgAt(Date.now());
    });
    return () => offEvent();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [partnerId]);

  // Track the hash so the Sidebar highlights Início/Chat/Configurações correctly.
  useEffect(() => {
    const onHash = () => setHash(window.location.hash);
    window.addEventListener("hashchange", onHash);
    return () => window.removeEventListener("hashchange", onHash);
  }, []);

  // Refresh the "há X min" labels once a minute.
  useEffect(() => {
    const h = window.setInterval(() => setNow(Date.now()), 60_000);
    return () => clearInterval(h);
  }, []);

  const presenceLabel =
    presence === "online" ? "Online" : presence === "away" ? "Ausente" : "Offline";

  // Card 2 — partner's live activity (real; reuses the lib mapping).
  const game = detectGame(partnerActivity);
  const activityValue = game
    ? `Jogando ${game}`
    : partnerActivity
      ? friendlyName(partnerActivity)
      : "Nenhuma atividade";

  // Feature 4 — resolve the partner's cached app icon reactively (cache-only;
  // the receiver never extracts the partner's exe). 24px in the activity card.
  // A late activity_icon push warms the cache and this re-resolves live.
  const partnerActivityIcon = useAppIcon(partnerActivity ?? "");
  const activityChip =
    partnerActivity && partnerActivityIcon ? (
      <img
        src={partnerActivityIcon}
        alt=""
        className="h-5 w-5 rounded object-contain"
        draggable={false}
      />
    ) : partnerActivity ? (
      <GeneratedAppIcon exe={partnerActivity} size={20} />
    ) : (
      <Gamepad2 className="h-5 w-5" />
    );

  // Card 4 — last interaction label (real; from local history + live events).
  const lastInteraction =
    lastMsgAt != null
      ? agoLabel(Math.max(0, Math.floor((now - lastMsgAt) / 1000)))
      : "Sem mensagens";

  return (
    <div className="window-main relative flex h-screen overflow-hidden">
      <OceanBackground />

      <Sidebar
        identity={identity}
        connected={connected === "open"}
        currentHash={hash}
        partnerActive={!!partnerActivity}
        myActivity={myActivity}
        shareActivity={shareActivity}
      />

      {/* Main column. The CallStrip sits at the top — the always-on call surface,
          never taking the column over. The partner hero + 2×2 info grid stay
          visible below it. The ActionBar anchors at the bottom right (just
          "Abrir chat" now — the call has no start/hang-up). */}
      <div className="flex h-full flex-1 flex-col gap-5 overflow-y-auto p-6">
        <CallStrip identity={identity} presence={presence} />

        <PartnerHeroCard
          partnerName={identity.partner_name || ""}
          partnerAvatar={identity.partner_avatar ?? null}
          presence={presence}
          presenceLabel={presenceLabel}
        />

        {/* 2×2 info grid */}
        <div className="grid grid-cols-2 gap-4">
          <InfoStatCard
            icon={<Clock className="h-5 w-5" />}
            label="Juntos hoje"
            value={togetherLabel(togetherSeconds)}
          />
          <InfoStatCard
            icon={activityChip}
            label="Atividade"
            value={activityValue}
          />
          <VoiceStatusCard />
          <InfoStatCard
            icon={<MessageSquare className="h-5 w-5" />}
            label="Última interação"
            value={lastInteraction}
          />
        </div>

        {/* Right-aligned action bar pinned to the bottom of the column. The
            quick-message presets moved to the dedicated "Notificações" tab —
            Home is now about presence + activity only. */}
        <div className="mt-auto pt-2">
          <ActionBar connected={connected === "open"} />
        </div>
      </div>
    </div>
  );
}
