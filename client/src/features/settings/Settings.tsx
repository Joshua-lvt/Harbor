/**
 * Settings — relay URL, display name, activity-mirroring toggle, notification
 * toggles, away timeout, autostart on boot, and unpair ("Desvincular").
 * Persists to the Tauri store (settings.json / identity.json) and applies the
 * autostart plugin immediately. Unpair hits the relay (which notifies the
 * ex-partner over their live WS) and then returns the user to the pairing
 * screen with a freshly reissued code so they can pair with someone new.
 */
import { useEffect, useRef, useState } from "react";
import type { Identity, Settings } from "../../lib/types";
import { saveIdentity, saveSettings, loadSettings } from "../../lib/identity";
import { setProfile } from "../../lib/relay";
import { socket } from "../../services/ws";
import { setAutostart } from "../../services/autostart";
import { fileToAvatarDataUrl } from "../../lib/image";
import { SharkMascot } from "../../assets/shark";
import { Avatar } from "../../components/Avatar";

export default function SettingsScreen({
  identity,
  settings,
  onSettings,
  onIdentity,
  onUnpair,
  back,
}: {
  identity: Identity;
  settings: Settings;
  onSettings: (s: Settings) => void;
  onIdentity: (next: Identity) => void;
  onUnpair: () => Promise<void>;
  back: () => void;
}) {
  const [relayUrl, setRelayUrl] = useState(identity.relay_url);
  const [name, setName] = useState(identity.my_name ?? "");
  const [avatar, setAvatar] = useState(identity.my_avatar ?? "");
  const [s, setS] = useState(settings);
  const [autostartErr, setAutostartErr] = useState("");
  const [saved, setSaved] = useState("");
  const [unpairing, setUnpairing] = useState(false);
  const [unpairErr, setUnpairErr] = useState("");
  const [avatarBusy, setAvatarBusy] = useState(false);
  const [avatarErr, setAvatarErr] = useState("");
  // The relay URL is hidden by default — it's an advanced/private setting the
  // user doesn't want shown. Revealed only via the "Avançado" toggle.
  const [showAdvanced, setShowAdvanced] = useState(false);
  const avatarInput = useRef<HTMLInputElement>(null);

  useEffect(() => {
    loadSettings().then(setS);
  }, []);

  async function onPickAvatar(files: FileList | null) {
    if (!files || !files[0]) return;
    setAvatarBusy(true);
    setAvatarErr("");
    try {
      const dataUrl = await fileToAvatarDataUrl(files[0]);
      setAvatar(dataUrl);
      // Publish immediately so the partner sees the new photo without a "Salvar"
      // round-trip — like the name field below, the photo is part of the profile.
      try {
        const credentialed: Identity = { ...identity, relay_url: relayUrl };
        await setProfile(credentialed, name.trim() || null, undefined, dataUrl);
        const nextId: Identity = { ...identity, my_avatar: dataUrl };
        await saveIdentity(nextId);
        onIdentity(nextId);
        // Real-time push so an online partner updates live (the HTTP POST
        // /profile remains the persistent source of truth; this is the live
        // opt). Wrapped — the socket may be closed (deployed Worker pre-deploy
        // drops it as an unknown type the client ignores).
        try {
          socket.send({
            type: "profile_update",
            display_name: name.trim() || null,
            avatar: dataUrl,
          });
        } catch {
          /* socket closed — syncs on next cold start via HTTP */
        }
      } catch {
        // Relay unreachable — keep the change locally (persisted on Salvar).
        setAvatarErr("Salvo localmente (relay indisponível).");
      }
    } catch (e) {
      setAvatarErr(e instanceof Error ? e.message : "Falha ao processar a imagem.");
    } finally {
      setAvatarBusy(false);
      if (avatarInput.current) avatarInput.current.value = "";
    }
  }

  async function clearAvatar() {
    setAvatar("");
    try {
      const credentialed: Identity = { ...identity, relay_url: relayUrl };
      // "" clears the stored photo server-side (distinct from undefined = keep).
      await setProfile(credentialed, name.trim() || null, undefined, "");
      const nextId: Identity = { ...identity, my_avatar: null };
      await saveIdentity(nextId);
      onIdentity(nextId);
      // Live push the avatar clear (null = a real clear here — this path fires
      // only on an explicit "Remover" click). HTTP /profile stays the source of
      // truth; the WS push is the real-time optimization.
      try {
        socket.send({ type: "profile_update", display_name: name.trim() || null, avatar: null });
      } catch {
        /* socket closed — syncs on next cold start via HTTP */
      }
    } catch {
      // Relay unreachable — keep the local clear; persists on Salvar.
    }
  }

  async function saveAll() {
    const next: Settings = { ...s, relay_url: relayUrl };
    await saveSettings(next);
    onSettings(next);
    const nextId: Identity = {
      ...identity,
      relay_url: relayUrl,
      my_name: name.trim() || null,
      my_avatar: avatar || null,
    };
    await saveIdentity(nextId);
    onIdentity(nextId);
    if (name.trim()) {
      try {
        await setProfile({ ...identity, relay_url: relayUrl }, name.trim());
      } catch {
        // Relay may be unreachable; still keep the change locally.
      }
    }
    // Real-time push of the full saved profile so an online partner updates
    // live (name + avatar together). HTTP /profile stays the source of truth.
    // Wrapped — the deployed Worker (pre-deploy) drops the type as unknown
    // and the client ignores the error; sync falls back to next cold start.
    try {
      socket.send({
        type: "profile_update",
        display_name: name.trim() || null,
        avatar: avatar || null,
      });
    } catch {
      /* socket closed — syncs on next cold start via HTTP */
    }
    setSaved("Salvo!");
    window.setTimeout(() => setSaved(""), 2000);
  }

  function toggle<K extends keyof Settings>(k: K) {
    setS((prev) => ({ ...prev, [k]: !prev[k] }));
  }

  /** Toggle the floating widget immediately — App's lifecycle effect opens /
   *  closes the window once `widget_enabled` flips. Persisting here (not on
   *  Salvar) matches the theme pattern: the window appears/disappears now.
   *  Disabled until paired, since the widget is about the partner. */
  async function toggleWidget() {
    if (!identity.partner_id) return;
    const next: Settings = { ...s, widget_enabled: !s.widget_enabled };
    setS(next);
    await saveSettings(next);
    onSettings(next);
  }

  /** Toggle "Iniciar com o Windows" IMMEDIATELY — register/unregister with the
   *  OS now (no Salvar round-trip), persist the setting, and lift to App so its
   *  boot-reconcile effect keeps the Windows Run key in sync with this choice
   *  across launches. Mirrors `toggleWidget`. If the autostart plugin isn't
   *  available on this target (non-desktop / capability missing / plugin init
   *  failed at startup), the OS call throws — we revert the setting + checkbox
   *  so the UI honestly reflects that the OS won't auto-launch, and surface the
   *  error inline. This is the fix for the old silent-failure path where the
   *  toggle was deferred to Salvar AND any enable() error was swallowed, so
   *  "Salvo!" showed but nothing was ever registered. */
  async function toggleAutostart() {
    const next = !s.autostart_enabled;
    const ns: Settings = { ...s, autostart_enabled: next };
    setS(ns);
    onSettings(ns);
    await saveSettings(ns);
    try {
      await setAutostart(next);
      setAutostartErr("");
    } catch {
      const reverted: Settings = { ...s };
      setS(reverted);
      onSettings(reverted);
      await saveSettings(reverted);
      setAutostartErr("Não foi possível configurar o inicializar — plugin indisponível neste sistema.");
      window.setTimeout(() => setAutostartErr(""), 5000);
    }
  }

  async function doUnpair() {
    const ok = window.confirm(
      "Desvincular parceiro?\n\nIsso encerra a conexão atual. O histórico de chat com esta pessoa será apagado localmente, e ambos poderão parear com outras pessoas. Esta ação não pode ser desfeita.",
    );
    if (!ok) return;
    setUnpairing(true);
    setUnpairErr("");
    try {
      await onUnpair();
    } catch (e) {
      setUnpairErr(e instanceof Error ? e.message : String(e));
    } finally {
      setUnpairing(false);
    }
  }

  return (
    <div className="window-main h-screen flex flex-col">
      <div className="flex items-center gap-3 px-4 py-3 border-b border-harbor-line bg-harbor-surface/60 backdrop-blur">
        <button onClick={back} className="text-sm text-harbor-deep">← Voltar</button>
        <span className="font-semibold text-harbor-deep">Configurações</span>
      </div>

      <div className="flex-1 overflow-y-auto p-4 flex flex-col gap-5">
        <section className="flex flex-col gap-2">
          <p className="text-xs font-medium text-harbor-deep/70">Perfil</p>
          <div className="flex items-center gap-4 rounded-xl bg-harbor-surface/70 px-3 py-3">
            <Avatar src={avatar || null} alt={name || "Você"} size={64} />
            <div className="flex flex-col gap-2">
              <input
                ref={avatarInput}
                type="file"
                accept="image/*"
                className="hidden"
                onChange={(e) => void onPickAvatar(e.target.files)}
              />
              <div className="flex gap-2">
                <button
                  type="button"
                  onClick={() => avatarInput.current?.click()}
                  disabled={avatarBusy}
                  className="rounded-lg bg-harbor-sea px-3 py-1.5 text-xs font-medium text-white hover:bg-harbor-deep transition disabled:opacity-50"
                >
                  {avatarBusy ? "Processando…" : "Escolher foto"}
                </button>
                {avatar && (
                  <button
                    type="button"
                    onClick={clearAvatar}
                    disabled={avatarBusy}
                    className="rounded-lg border border-red-300/70 bg-red-50 px-3 py-1.5 text-xs font-medium text-red-700 hover:bg-red-100 dark:border-red-400/30 dark:bg-red-500/15 dark:text-red-300 dark:hover:bg-red-500/25 transition disabled:opacity-50"
                  >
                    Remover
                  </button>
                )}
              </div>
              {avatarErr && <p className="text-[11px] text-red-700 dark:text-red-300">{avatarErr}</p>}
            </div>
          </div>
          <label className="mt-1 text-xs font-medium text-harbor-deep/70">Seu nome</label>
          <input
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder="(opcional)"
            className="rounded-xl border border-harbor-sky bg-harbor-surface-strong px-3 py-2 text-sm outline-none focus:border-harbor-sea"
          />
          <p className="text-[11px] text-harbor-ink/50">
            Sua foto e nome aparecem para o parceiro. A foto é compactada e enviada ao relay (como a chave pública) para ele ver.
          </p>
        </section>

        <section className="flex flex-col gap-2">
          <button
            type="button"
            onClick={() => setShowAdvanced((v) => !v)}
            className="flex items-center justify-between rounded-xl bg-harbor-surface/70 px-3 py-2 text-sm font-medium text-harbor-ink transition hover:bg-harbor-surface"
          >
            <span>Avançado</span>
            <span className="text-xs text-harbor-ink/50">{showAdvanced ? "▲" : "▼"}</span>
          </button>
          {showAdvanced && (
            <div className="flex flex-col gap-2">
              <label className="text-xs font-medium text-harbor-deep/70">Relay (wss:// recomendado)</label>
              <input
                value={relayUrl}
                onChange={(e) => setRelayUrl(e.target.value)}
                className="rounded-xl border border-harbor-sky bg-harbor-surface-strong px-3 py-2 text-sm outline-none focus:border-harbor-sea"
              />
              <p className="text-[11px] text-harbor-ink/50">
                Servidor privado seu e do seu parceiro — não é nosso. Use wss:// e TLS em qualquer rede que não seja local.
              </p>
            </div>
          )}
        </section>

        <section className="flex flex-col gap-2">
          <p className="text-xs font-medium text-harbor-deep/70">Atividade</p>
          <label className="flex items-center justify-between rounded-xl bg-harbor-surface/70 px-3 py-2">
            <span className="text-sm text-harbor-ink">Compartilhar meu app atual</span>
            <input
              type="checkbox"
              checked={s.share_activity}
              onChange={() => toggle("share_activity")}
              className="h-4 w-4 accent-harbor-deep"
            />
          </label>
          <p className="text-[11px] text-harbor-ink/50">
            Mostra ao parceiro o app que você está usando ("💻 Usando …" /
            "🎮 Jogando …"), estilo Discord. Só o nome do processo — nunca o
            título da janela ou o conteúdo da tela. Desligue quando quiser
            privacidade total.
          </p>
        </section>

        <section className="flex flex-col gap-2">
          <p className="text-xs font-medium text-harbor-deep/70">Notificações</p>
          {([
            ["notify_on_online", "Parceiro ficou online 💙"],
            ["notify_on_away", "Parceiro ficou ausente 🌙"],
            ["notify_on_offline", "Parceiro ficou offline ⚫"],
            ["notify_on_message", "Nova mensagem 💬"],
            ["notif_sound_enabled", "Som nas notificações Harbor 🔔"],
          ] as const).map(([key, label]) => (
            <label key={key} className="flex items-center justify-between rounded-xl bg-harbor-surface/70 px-3 py-2">
              <span className="text-sm text-harbor-ink">{label}</span>
              <input
                type="checkbox"
                checked={s[key]}
                onChange={() => toggle(key)}
                className="h-4 w-4 accent-harbor-deep"
              />
            </label>
          ))}
        </section>

        <section className="flex flex-col gap-2">
          <label className="text-xs font-medium text-harbor-deep/70">
            Ficar "ausente" após: {s.away_after_minutes} min
          </label>
          <input
            type="range"
            min={1}
            max={30}
            value={s.away_after_minutes}
            onChange={(e) => setS((prev) => ({ ...prev, away_after_minutes: Number(e.target.value) }))}
            className="accent-harbor-deep"
          />
        </section>

        <section className="flex items-center justify-between rounded-xl bg-harbor-surface/70 px-3 py-2">
          <div>
            <p className="text-sm text-harbor-ink">Iniciar com o Windows</p>
            <p className="text-[11px] text-harbor-ink/50">Abre automaticamente no boot.</p>
          </div>
          <input
            type="checkbox"
            checked={s.autostart_enabled}
            onChange={() => void toggleAutostart()}
            className="h-4 w-4 accent-harbor-deep"
          />
        </section>
        {autostartErr && (
          <p className="-mt-2 text-[11px] text-red-700 dark:text-red-300">{autostartErr}</p>
        )}

        <section className="flex flex-col gap-2 rounded-xl bg-harbor-surface/70 px-3 py-2">
          <label className="flex items-center justify-between">
            <div>
              <p className="text-sm text-harbor-ink">Widget flutuante</p>
              <p className="text-[11px] text-harbor-ink/50">
                Fica fixado sobre os outros apps; mostra se o parceiro está online
                e qual app ele usa.
              </p>
            </div>
            <input
              type="checkbox"
              checked={s.widget_enabled}
              onChange={() => void toggleWidget()}
              className="h-4 w-4 accent-harbor-deep"
            />
          </label>
        </section>

        <div className="flex items-center justify-between mt-2">
          <SharkMascot className="w-8 h-8 opacity-70" />
          <div className="flex items-center gap-3">
            {saved && <span className="text-xs text-harbor-deep">{saved}</span>}
            <button
              onClick={saveAll}
              className="rounded-xl bg-harbor-deep text-white px-4 py-2 text-sm font-medium hover:bg-harbor-sea transition"
            >
              Salvar
            </button>
          </div>
        </div>

        <section className="flex flex-col gap-2 mt-4 pt-4 border-t border-harbor-line">
          <p className="text-xs font-medium text-harbor-deep/70">Conexão</p>
          <p className="text-[11px] text-harbor-ink/50">
            Desvincula o parceiro atual. O histórico de chat com esta pessoa é
            apagado, e ambos podem parear com outras pessoas a partir de agora.
          </p>
          <button
            onClick={doUnpair}
            disabled={unpairing || !identity.partner_id}
            className="rounded-xl border border-red-300/70 bg-red-50 px-4 py-2 text-sm font-medium text-red-700 hover:bg-red-100 dark:border-red-400/30 dark:bg-red-500/15 dark:text-red-300 dark:hover:bg-red-500/25 disabled:opacity-50 transition"
          >
            {unpairing ? "Desvinculando…" : "Desvincular parceiro"}
          </button>
          {unpairErr && (
            <p className="text-[11px] text-red-700 dark:text-red-300">
              Não foi possível desvincular: {unpairErr}
            </p>
          )}
        </section>
      </div>
    </div>
  );
}
