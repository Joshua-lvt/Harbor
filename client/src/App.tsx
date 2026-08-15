/**
 * App entry for the Harbor main window.
 *
 * Hash routing (no router dependency): the route is parsed from
 * `window.location.hash`. One entry, one build. The socket is a singleton
 * owned by this window and stays connected across route changes
 * (settings ↔ chat) and across close-to-tray (a hidden window keeps its
 * webview alive, so presence stays online in the background).
 */
import { useEffect, useRef, useState, type ReactNode } from "react";
import PairingScreen from "./features/pairing/PairingScreen";
import ChatScreen from "./features/chat/ChatScreen";
import HomeScreen from "./features/home/HomeScreen";
import SettingsScreen from "./features/settings/Settings";
import ActivitiesScreen from "./features/activity/ActivitiesScreen";
import { ensureIdentity, loadIdentity, loadSettings, saveIdentity, saveSettings } from "./lib/identity";
import { genKeypair } from "./lib/crypto";
import { socket } from "./services/ws";
import { attachNotifications } from "./services/notify";
import { attachTrayPresence } from "./services/tray";
import { useActivity } from "./hooks/useActivity";
import { useToast } from "./components/Toaster";
import { detectGame } from "./lib/appNames";
import { getMe, getPartner, setProfile, unpair as relayUnpair, type PartnerInfo } from "./lib/relay";
import { clearAllMessages } from "./lib/localDb";
import { ActivityTracker } from "./lib/activityDerivation";
import { clearActivityHistory } from "./lib/activityHistory";
import { storePartnerIcon, peekIcon } from "./lib/appIconCache";
import { voice } from "./services/voice";
import { DEFAULT_SETTINGS } from "./lib/types";
import type { Identity, PresenceState, Settings, ServerEvent } from "./lib/types";
import { applyTheme } from "./lib/theme";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useHarborNotifications } from "./hooks/useHarborNotifications";
import { NOTIF_NAVIGATE_EVENT } from "./services/notifOverlay";
import {
  openWidget,
  closeWidget,
  emitWidgetState,
  widgetExists,
  WIDGET_CLOSED_EVENT,
  type WidgetSnapshot,
} from "./services/widget";
import type { VoiceState } from "./services/voice";

type Route = "pairing" | "home" | "chat" | "settings" | "activity";

function getRoute(): Route {
  if (window.location.hash.startsWith("#/settings")) return "settings";
  if (window.location.hash.startsWith("#/chat")) return "chat";
  if (window.location.hash.startsWith("#/activity")) return "activity";
  if (window.location.hash.startsWith("#/home")) return "home";
  return "pairing";
}

export default function App() {
  const [route, setRoute] = useState<Route>(getRoute);
  const [id, setId] = useState<Identity | null>(null);
  const [settings, setSettings] = useState<Settings>(DEFAULT_SETTINGS);
  // Socket status drives the outbound-activity broadcaster (`useActivity`) so
  // my foreground app only sends when actually connected. App-lifetime state.
  const [connected, setConnected] = useState(socket.getStatus());
  // Partner's last advertised foreground app (lowercased exe, e.g. "code.exe"),
  // shown live on Home. Updated from `activity` events (see effect below).
  const [partnerActivity, setPartnerActivity] = useState<string | null>(null);
  // Partner activity's cached icon (resolve-by-exe of partnerActivity). Looked
  // up synchronously from the icon cache so buildSnapshot can carry it without
  // awaiting — null → the widget/partner UI renders a generated fallback. Bumped
  // by `iconTick` whenever a new `activity_icon` push warms the cache so the
  // reconciler re-emits + Home re-resolves.
  const [partnerActivityIcon, setPartnerActivityIcon] = useState<string | null>(null);
  const [iconTick, setIconTick] = useState(0);
  // Partner's presence at the app level — the widget snapshot needs it (the
  // widget owns no socket), so unlike Home/Chat (which keep their own local
  // presence state) we track it here once and feed both Home + the snapshot.
  const [partnerPresence, setPartnerPresence] = useState<PresenceState>("offline");
  // Snapshot of the voice singleton, kept app-lifetime so the widget reconciler
  // re-emits when call status changes without each screen re-wiring.
  const [voiceState, setVoiceState] = useState<VoiceState>(voice.getState());

  const { push } = useToast();
  const pushRef = useRef(push);
  useEffect(() => {
    pushRef.current = push;
  }, [push]);
  // Feature 5: drive Harbor quick-message notifications for the app lifetime —
  // in-app Toaster when the main window is focused, the separate always-on-top
  // `notif-overlay` window when it isn't. Mounted once; the hook re-subscribes
  // on settings change (cheap, keeps its closures current).
  useHarborNotifications(settings);
  // Toast body uses the friendly partner name; keep a ref so the activity
  // subscriber effect (deps []) can read the latest without re-subscribing.
  const partnerNameRef = useRef("Seu parceiro");
  useEffect(() => {
    partnerNameRef.current = id?.partner_name || "Seu parceiro";
  }, [id]);
  // Current partner activity (exe) mirrored into a ref so the app-lifetime
  // `activity_icon` handler (deps []) can compare the pushed exe against the
  // live value without re-subscribing on every `partnerActivity` change.
  const partnerActivityRef = useRef<string | null>(null);
  useEffect(() => {
    partnerActivityRef.current = partnerActivity;
    // Whenever the current activity changes, re-resolve its icon from the
    // cache (a partner may have pushed it earlier; peekIcon returns null
    // synchronously when not yet warmed → a later activity_icon push bump
    // corrects it). This is the *receiver* resolve (cache-only, no path).
    setPartnerActivityIcon(partnerActivity ? peekIcon(partnerActivity) : null);
  }, [partnerActivity]);

  // Holds the active notification + tray subscription unsubscribe handles, so
  // any re-attach (post-pair, StrictMode remount) first detaches the prior
  // copies — which otherwise leak and fire duplicate notifications on every
  // presence transition. Both onPaired and the boot effect push into here.
  const notifOffRef = useRef<Array<() => void>>([]);
  const detachNotifs = () => {
    notifOffRef.current.forEach((off) => off());
    notifOffRef.current = [];
  };

  // Tear down a dead pairing and return to the pairing screen so the user can
  // link with someone new. Called from: (a) the "Desvincular" button in Settings
  // (`viaPartner=false` — it was MY choice), (b) an inbound `unpaired` WS event
  // (`viaPartner=true` — the partner initiated it; we got our reissued code in
  // the envelope), and (c) boot recovery when /partner 404s (the partner
  // unpaired us while we were offline; we resync the code from /me). Ref so the
  // app-lifetime WS subscriber can call the latest without re-subscribing.
  const unpairRef = useRef<(code: string | null, viaPartner: boolean) => Promise<void>>(
    async () => {},
  );
  unpairRef.current = async (code: string | null, viaPartner: boolean) => {
    // Read the freshest identity from the store (the WS-event path can fire for
    // a stale `id` react state).
    const current = await loadIdentity();
    if (!current) return;
    const next: Identity = {
      ...current,
      partner_id: null,
      partner_name: null,
      pairing_code: code ?? current.pairing_code ?? null,
    };
    socket.close(); // stop talking to the dead link (also drops presence)
    voice.stopAlwaysOn(); // tear down the always-on call + release the mic
    detachNotifs(); // drop the notification/tray subscribers for the ex-partner
    await saveIdentity(next);
    await clearAllMessages(); // fresh slate — don't show an ex's history
    await clearActivityHistory(); // Feature 3: wipe the ex's activity history
    activityTrackerRef.current?.reset();
    setId(next);
    if (getRoute() !== "pairing") window.location.hash = "#/pairing";
    if (viaPartner) {
      pushRef.current({
        icon: "💔",
        title: "Harbor",
        body: "Seu parceiro desvinculou a conexão. Você pode parear com outra pessoa agora.",
        duration: 6000,
      });
    }
  };

  useEffect(() => {
    const onHash = () => setRoute(getRoute());
    window.addEventListener("hashchange", onHash);
    return () => window.removeEventListener("hashchange", onHash);
  }, []);

  // Load identity + settings, then own the socket for the app lifetime when
  // already paired. The socket persists across route changes and close-to-tray.
  useEffect(() => {
    let cancelled = false;
    (async () => {
      const s = await loadSettings();
      if (!cancelled) setSettings(s);
      const existing = await loadIdentity();
      if (cancelled) return;

      if (existing?.partner_id && existing?.device_secret) {
        // Validate the link still exists server-side — the partner may have
        // unpaired us while we were offline. If /partner 404s, the link is
        // already gone on the relay; resync our reissued code from /me and
        // return to pairing instead of connecting a dead link.
        //
        // /partner ALSO hands back the partner's current X25519 public key
        // (the passive partner learns the caller's key here, since only the
        // caller hits /pair). We load it into our identity so the chat can seal
        // to them. For an already-paired install that predates E2E this branch
        // also publishes THIS device's own key (the upgrade path — avoids
        // re-/register, which would re-issue the device_secret and force a
        // socket re-auth). All before the socket connects, so the secret +
        // send envelope are stable on the first WS frame.
        let partner: PartnerInfo;
        try {
          partner = await getPartner(existing);
        } catch {
          try {
            const me = await getMe(existing);
            await unpairRef.current(me.pairing_code, true);
          } catch {
            await unpairRef.current(null, true);
          }
          return;
        }
        let id = existing;
        // Upgrade: an install whose store predates E2E has no device keys
        // yet. Generate them now and publish the pubkey via /profile (keeps
        // the device_secret stable — no re-auth), so the partner can pick it
        // up on THEIR next cold start. Matters even when partner has no key
        // yet: ours should be published regardless, so we're ready the moment
        // they publish theirs.
        if (!id.device_privkey || !id.device_pubkey) {
          try {
            const kp = await genKeypair();
            await setProfile(id, id.my_name ?? null, kp.pub);
            id = { ...id, device_pubkey: kp.pub, device_privkey: kp.priv };
          } catch {
            // Relay unreachable / profile rejected — proceed with no keys;
            // the client falls back to plaintext (visibly marked ⚠). We don't
            // block the app on a crypto publish; the chat still works.
          }
        }
        // Refresh the partner's pubkey in case it changed (they reinstalled /
        // re-keyed) while we were offline, or it simply wasn't cached yet.
        const partnerPub = partner.partner_public_key ?? null;
        if (partnerPub !== (id.partner_pubkey ?? null)) {
          id = { ...id, partner_pubkey: partnerPub };
        }
        // Same refresh for the partner's avatar — they may have set/changed it
        // while we were offline (or it simply wasn't cached yet on this install).
        const partnerAvatar = partner.partner_avatar ?? null;
        if (partnerAvatar !== (id.partner_avatar ?? null)) {
          id = { ...id, partner_avatar: partnerAvatar };
        }
        await saveIdentity(id);
        setId(id);
        // Feature 3: spin up the activity-history tracker for this partner + a
        // fresh derived state (no cross-pair leakage). reset() guards against
        // a StrictMode remount leaving a dangling prevExe.
        activityTrackerRef.current = new ActivityTracker(id.partner_id!);
        // Seed app-level presence from REST — the widget snapshot + Home both
        // need it; Home used to fetch its own copy, but the widget has no socket,
        // so the canonical app-level value starts here and is kept live by the
        // socket subscriber above.
        setPartnerPresence(partner.presence as PresenceState);
        // Already paired: own the socket + notifications for the app lifetime.
        socket.connect(id.relay_url, id.device_id, id.device_secret);
        detachNotifs();
        notifOffRef.current.push(attachNotifications((h) => socket.onEvent(h), s, id.partner_name ?? null));
        notifOffRef.current.push(attachTrayPresence((h) => socket.onEvent(h)));
        // Engage the always-on voice call. Role is deterministic (the smaller
        // device_id is the offerer) so both sides never offer at once. This is
        // idempotent against re-runs of the boot effect; `startAlwaysOn` guards
        // on its own `running` flag.
        voice.startAlwaysOn((id.device_id ?? "") < (id.partner_id ?? ""));
        if (getRoute() === "pairing") window.location.hash = "#/home";
      } else {
        // Not paired yet: ensure a device id exists, stay on pairing.
        setId(await ensureIdentity(s.relay_url));
      }
    })();
    // The socket persists for the app lifetime; do not close on unmount. Detach
    // only the per-run notifications/tray subscriptions so a re-run (StrictMode
    // dev double-mount, or any future re-run) doesn't double-attach them and
    // fire duplicate presence notifications ("ficou online" spam once per copy).
    return () => {
      cancelled = true;
      detachNotifs();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // App-lifetime socket status + inbound partner-activity/presence handling.
  // Subscribed once (deps []) and read latest values through refs so it doesn't
  // re-subscribe on every render / toast / identity change. This owns the
  // RECEIVER side of the foreground-app feature that `useActivity` broadcasts,
  // and app-level presence (the widget snapshot needs presence without owning a
  // socket). Voice state is mirrored the same way for the snapshot (the widget
  // never imports the voice singleton).
  const prevGameRef = useRef<string | null>(null);
  // Feature 3: one ActivityTracker per partner session, converts the inbound
  // `activity` event stream into persisted chronological history rows. Owned
  // app-lifetime (set in the boot effect when paired; reset/cleared on unpair).
  const activityTrackerRef = useRef<ActivityTracker | null>(null);
  useEffect(() => {
    const offStatus = socket.onStatus(setConnected);
    const offVoice = voice.onState(setVoiceState);
    const offEvent = socket.onEvent(async (e: ServerEvent) => {
      if (e.type === "activity") {
        const exe = e.app ?? null;
        setPartnerActivity(exe);
        // Feature 3: derive a persisted history row from this transition. The
        // tracker is best-effort + non-blocking — never gate the live UI on it.
        void activityTrackerRef.current?.onActivity(exe, e.ts * 1000);
        const game = detectGame(exe);
        // Toast only when a NEW game gains focus — never for leaving a game or
        // switching between non-game apps (those just update the badge).
        if (game && game !== prevGameRef.current) {
          // Feature 4: attach the cached partner-app icon to the game-start
          // toast if one is available (the overlay/img rendering site picks it
          // up; absent → just the emoji, no behavior change).
          const ico = exe ? peekIcon(exe) : null;
          pushRef.current({
            icon: "🎮",
            title: "Harbor",
            body: `${partnerNameRef.current} está jogando ${game}`,
            duration: 5000,
            ...(ico ? { image: ico } : {}),
          });
        }
        prevGameRef.current = game;
      } else if (e.type === "presence") {
        setPartnerPresence(e.state);
      } else if (e.type === "last_seen") {
        setPartnerPresence(e.presence);
      } else if (e.type === "activity_icon") {
        // Feature 4 — receiver side: store the partner's once-per-exe icon push
        // (memory + SQLite). Staging the cache wakes any `useAppIcon` mounted
        // for this exe (Home card, Activities row, widget) so the icon swaps in
        // without a manual re-render. If this push is for the CURRENT partner
        // activity, also refresh the app-level resolved icon + tick the
        // reconciler so the widget snapshot re-emits with the new icon.
        storePartnerIcon(e.app, e.icon);
        if (e.app === (partnerActivityRef.current ?? "")) {
          setPartnerActivityIcon(e.icon);
          setIconTick((t) => t + 1);
        }
      } else if (e.type === "profile_update") {
        // Real-time push of the partner's name +/or avatar (Feature 1). The HTTP
        // /profile is the persistent source of truth; this is the live opt so
        // an online partner stops seeing stale values. null = "no value" → skip
        // (matches the cold-start getPartner semantics), so a sender that omits
        // a field doesn't clobber our cached value with null. Wrapped: the WS
        // dispatch is sync, so an async handler must not leak an unhandled
        // rejection (a failed store read here is non-fatal — next cold-start
        // HTTP re-syncs).
        try {
          const current = await loadIdentity();
          if (!current) return;
          const next: Identity = { ...current };
          let changed = false;
          if (e.display_name != null && e.display_name !== (current.partner_name ?? null)) {
            next.partner_name = e.display_name;
            changed = true;
          }
          if (e.avatar != null && e.avatar !== (current.partner_avatar ?? null)) {
            next.partner_avatar = e.avatar;
            changed = true;
          }
          if (changed) {
            await saveIdentity(next);
            setId(next);
          }
        } catch {
          /* store failure — non-fatal; cold-start HTTP re-syncs */
        }
      } else if (e.type === "unpaired") {
        // The partner broke the pairing from their side. `e.pairing_code` is OUR
        // freshly reissued code — tear down the dead link + return to pairing.
        void unpairRef.current(e.pairing_code, true);
      }
    });
    return () => {
      offStatus();
      offVoice();
      offEvent();
    };
  }, []);

  // Outbound: poll my own foreground app every ~4s. Sharing (settings) gates
  // the broadcast only — the local preview still runs so Home shows what I'm
  // actually using. App-level (not per-screen) so switching routes doesn't
  // reset the broadcast + cause a spurious re-send.
  const { activity: myActivity, activityPath: myActivityPath } = useActivity(connected === "open", settings.share_activity);

  // Re-apply the theme whenever the saved setting changes (Settings screen
  // toggle, or loadSettings at boot). main.tsx paints an initial guess before
  // React mounts to avoid a light flash; this settles it to the real value,
  // and — for "system" — keeps the OS listener attached so a Windows theme
  // change re-themes the app live.
  useEffect(() => applyTheme(settings.theme), [settings.theme]);

  // Widget lifecycle: open/close the always-on-top window to mirror the saved
  // toggle. Only when paired — the widget is about the partner, so it makes no
  // sense on the pairing screen, and refuses to open if the window already
  // exists (e.g. it was open and the user re-toggled on). When the partner
  // unpaired us we surface to pairing, where `id.partner_id` is null and this
  // effect closes any leftover window.
  useEffect(() => {
    const want = settings.widget_enabled && !!id?.partner_id;
    let cancelled = false;
    (async () => {
      const exists = await widgetExists();
      if (cancelled) return;
      if (want && !exists) {
        const w = await openWidget();
        // The new window mounts WidgetScreen, which subscribes to the state
        // event; emit the current snapshot now so it isn't blank until the next
        // change. (Safe to call even if openWidget returned null.)
        if (w) void emitNow();
      } else if (!want && exists) {
        await closeWidget();
      }
    })();
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [settings.widget_enabled, id?.partner_id]);

  // Widget reconciler: compose a snapshot from the app-lifetime state already
  // in App (presence, activity, voice, identity, theme) and emit it whenever any
  // input changes. The widget holds no socket + no store — this event is its
  // only source of truth. The dark flag resolves `system` against the OS so the
  // widget theme tracks the main window without the widget reading Settings.
  function resolvedDark(): boolean {
    return (
      settings.theme === "dark" ||
      (settings.theme === "system" &&
        window.matchMedia("(prefers-color-scheme: dark)").matches)
    );
  }

  function buildSnapshot(): WidgetSnapshot {
    return {
      connected: connected === "open",
      partnerPresence,
      partnerActivity,
      // Feature 4: resolved partner-app icon (cache-only; null → the widget
      // renders its generated fallback). Kept in sync by the iconTick bump on
      // every activity_icon push that targets the current activity.
      partnerActivityIcon,
      partnerName: id?.partner_name ?? "",
      partnerAvatar: id?.partner_avatar ?? null,
      myName: id?.my_name ?? "",
      myAvatar: id?.my_avatar ?? null,
      myActivity,
      voiceStatus: voiceState.status,
      dark: resolvedDark(),
    };
  }

  async function emitNow(): Promise<void> {
    await emitWidgetState(buildSnapshot());
  }

  useEffect(() => {
    if (!settings.widget_enabled || !id?.partner_id) return;
    void emitNow();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [
    settings.widget_enabled,
    settings.theme,
    id?.partner_id,
    id?.partner_name,
    id?.partner_avatar,
    id?.my_name,
    id?.my_avatar,
    connected,
    partnerPresence,
    partnerActivity,
    // Feature 4: a late activity_icon push (or a partner-activity transition)
    // may resolve the icon after first emit — re-emit so the widget swaps it in.
    partnerActivityIcon,
    iconTick,
    myActivity,
    voiceState,
  ]);

  // The widget's ✕ button emits `harbor-widget-closed`. The widget has no
  // store permission, so the main window owns Settings: flip widget_enabled to
  // false and persist (the lifecycle effect above then closes the window).
  useEffect(() => {
    let off: (() => void) | undefined;
    listen(WIDGET_CLOSED_EVENT, async () => {
      const next: Settings = { ...settings, widget_enabled: false };
      setSettings(next);
      await saveSettings(next);
    }).then((un) => {
      off = un;
    });
    return () => off?.();
    // settings is read fresh inside the handler via closure; deps would re-bind
    // on every toggle but that's cheap and keeps the closure correct.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [settings]);

  // Feature 5 — the notification overlay's "Responder" / "Abrir chat" emits
  // `harbor-navigate` (it's a separate JS realm and can't route itself). Focus
  // + unminimize the main window and jump to chat so the user lands ready to
  // type. Best-effort (non-Tauri / focus failures are non-fatal).
  useEffect(() => {
    let off: (() => void) | undefined;
    listen(NOTIF_NAVIGATE_EVENT, async () => {
      window.location.hash = "#/chat";
      try {
        const w = getCurrentWindow();
        await w.unminimize();
        await w.setFocus();
      } catch {
        /* non-Tauri — the hash route is the best we can do */
      }
    }).then((un) => {
      off = un;
    });
    return () => off?.();
  }, []);

  if (!id) return <div className="window-main h-screen" />;

  // The remote-voice <audio> lives at the app root (not inside a route) so a
  // call persists across route changes — open Chat mid-call and audio keeps
  // playing. The voice singleton (services/voice.ts) owns the RTCPeerConnection
  // and attaches its remote stream to this element.
  let content: ReactNode;
  if (route === "home" && id.partner_id)
    content = (
      <HomeScreen
        identity={id}
        partnerActivity={partnerActivity}
        myActivity={myActivity}
        myActivityPath={myActivityPath}
        shareActivity={settings.share_activity}
      />
    );
  else if (route === "settings")
    content = (
      <SettingsScreen
        identity={id}
        settings={settings}
        onSettings={setSettings}
        onIdentity={setId}
        onUnpair={async () => {
          // Hit the relay FIRST (the partner is notified over their live WS),
          // THEN tear down locally. A real relay/network error propagates to
          // Settings so it can show a message — we only unbind on success.
          // Exception: a 404 means we're ALREADY not paired on the relay
          // (stale-link desync — the partner or we unpaired while we were
          // offline, and the local store still thinks we're paired). The
          // desired end state is already true server-side, so treat it as
          // success and still tear down locally, resyncing our freshly
          // reissued pairing_code from /me so we can pair with someone new.
          if (!id) return;
          let pairingCode: string | null = null;
          let viaPartner = false;
          try {
            const r = await relayUnpair(id);
            pairingCode = r.pairing_code;
          } catch (e) {
            const msg = e instanceof Error ? e.message : String(e);
            if (!/404/.test(msg)) throw e;
            viaPartner = true; // someone already broke this link — not our click
            try {
              const me = await getMe(id);
              pairingCode = me.pairing_code;
            } catch {
              pairingCode = null; // /me failed too — tear down with what we have
            }
          }
          await unpairRef.current(pairingCode, viaPartner);
        }}
        back={() => (window.location.hash = "#/home")}
      />
    );
  else if (route === "chat" && id.partner_id) content = <ChatScreen identity={id} />;
  else if (route === "activity" && id.partner_id)
    content = (
      <ActivitiesScreen
        identity={id}
        back={() => (window.location.hash = "#/home")}
      />
    );
  else
    content = (
      <PairingScreen
        identity={id}
        onPaired={(next) => {
          setId(next);
          socket.connect(next.relay_url, next.device_id, next.device_secret);
          // Feature 3: start a fresh activity-history chain for the new partner.
          activityTrackerRef.current = new ActivityTracker(next.partner_id!);
          detachNotifs();
          notifOffRef.current.push(attachNotifications((h) => socket.onEvent(h), settings, next.partner_name ?? null));
          notifOffRef.current.push(attachTrayPresence((h) => socket.onEvent(h)));
          voice.startAlwaysOn((next.device_id ?? "") < (next.partner_id ?? ""));
          window.location.hash = "#/home";
        }}
      />
    );

  return (
    <>
      <VoiceAudio />
      {content}
    </>
  );
}

/** Hidden, always-mounted remote-audio sink for the voice call. */
function VoiceAudio() {
  const ref = useRef<HTMLAudioElement>(null);
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const detach = voice.attachAudioElement(el);
    return detach;
  }, []);
  return <audio ref={ref} autoPlay className="hidden" />;
}
