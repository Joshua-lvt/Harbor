/**
 * UpdateGate — the mandatory (non-dismissible) auto-update gate (Fase 2).
 *
 * Wraps the main window's <App/> (see main.tsx). The App — and therefore the
 * socket, identity, pairing, SQLite, and relay layers — only mounts once the
 * gate reaches `up-to-date`. In every other state the user is shown a
 * full-screen blocking screen with exactly one available action per state.
 * There is no "Depois" / "Adiar" / close-to-bypass button: when a newer build
 * exists, no older version may run, and a network/server failure is treated as
 * "could not confirm up-to-date" (still blocked) — never a silent pass.
 *
 * Lifecycle (production, `import.meta.env.PROD`):
 *   mount → `checking` → check() from @tauri-apps/plugin-updater
 *     ├─ null           → `up-to-date`  (renders children)
 *     ├─ Update object  → `available`   (shows current→new, "ATUALIZAR AGORA")
 *     │                    click → downloadAndInstall(…)
 *     │                      ├─ Started/Progress → `downloading`
 *     │                      ├─ Finished         → `installing` → relaunch()
 *     │                      └─ reject (download phase) → `download-error`
 *     │                         reject (install phase)  → `install-error`
 *     └─ check() reject  → `check-error`  (server/reachability problem)
 *   Any error state → "TENTAR NOVAMENTE" resets to `checking` and re-runs
 *   check() (download-side errors re-invoke downloadAndInstall instead).
 *
 * DEV (`import.meta.env.DEV`, i.e. `npm run tauri dev`): the gate is skipped —
 * it assumes `up-to-date` and logs once. Distributed builds are always
 * production (NSIS via GitHub Actions), where the gate acts rigorously. This
 * avoids locking local dev when no GitHub Release/manifest exists yet.
 *
 * The persisted-data survival guarantee needs no code here: identity/settings
 * live in %APPDATA%\com.harbor.app (tauri-plugin-store) and the SQLite db via
 * tauri-plugin-sql — both outside the install dir the NSIS installer replaces
 * — so identity, pairing, X25519 keys, and history survive an update untouched.
 * This gate never calls ensureIdentity (it must not regenerate identity).
 */
import { useCallback, useEffect, useState, type ReactNode } from "react";
import { check, type DownloadEvent, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { getVersion } from "@tauri-apps/api/app";

type GateState =
  | "checking"
  | "up-to-date"
  | "available"
  | "downloading"
  | "installing"
  | "download-error"
  | "install-error"
  | "check-error";

/**
 * Distinguish a downloadAndInstall rejection's phase. The updater plugin (and
 * the underlying Rust side) does not expose a typed error, so we heuristically
 * classify the message: if it mentions the install step it is an install
 * failure, otherwise it is a download/connectivity failure. Both leave the app
 * blocked — the distinction only chooses which message + retry path to show.
 */
function classifyInstallError(err: unknown): "download" | "install" {
  const msg = err instanceof Error ? err.message : String(err);
  return /install/i.test(msg) ? "install" : "download";
}

export default function UpdateGate({ children }: { children: ReactNode }) {
  const [state, setState] = useState<GateState>("checking");
  const [currentVersion, setCurrentVersion] = useState<string>("…");
  const [newVersion, setNewVersion] = useState<string>("");
  const [progressBytes, setProgressBytes] = useState<number>(0);
  const [progressTotal, setProgressTotal] = useState<number | null>(null);
  // Holds the Update object between "available" (waiting for click) and the
  // download attempt, and across a retry that re-uses the same check() result.
  // useRef-stored because downloadAndInstall is called from the click handler and
  // the error-retry handler, both of which need the live handle. The Resource
  // is torn down (close()) when a fresh check() produces a different one.
  const [update, setUpdate] = useState<Update | null>(null);

  // The current app version is read from Tauri (the bundle version in
  // tauri.conf.json) so there is a single real source — no manual mirror in a
  // third place to fall out of sync.
  useEffect(() => {
    void getVersion()
      .then(setCurrentVersion)
      .catch(() => setCurrentVersion("?"));
  }, []);

  /**
   * Run the updater's check() and branch on the result. On a real update being
   * available, stash the Update handle (for the later downloadAndInstall) and
   * record the pending version. A null result means "running the newest build"
   * and releases the gate. A rejection means the endpoint/reachability failed —
   * treated as `check-error` (STILL BLOCKED, never a silent pass).
   */
  const runCheck = useCallback(async (): Promise<void> => {
    setState("checking");
    setProgressBytes(0);
    setProgressTotal(null);
    try {
      const result = await check();
      if (!result) {
        setState("up-to-date");
        return;
      }
      setUpdate(result);
      setNewVersion(result.version);
      setState("available");
    } catch (err) {
      console.error("[UpdateGate] check() failed:", err);
      setState("check-error");
    }
  }, []);

  // Mount-time check. DEV skips it entirely — see header. The gate never
  // silently falls back to up-to-date on a network/server failure, so check()
  // errors land in `check-error` with a retry, not in `up-to-date`.
  useEffect(() => {
    if (import.meta.env.DEV) {
      console.log("[UpdateGate] dev mode — skipping update check (assumes up-to-date).");
      setState("up-to-date");
      return;
    }
    void runCheck();
  }, [runCheck]);

  /**
   * Download + install the staged update, mapping the updater's streamed
   * DownloadEvent onto the `downloading`/`installing` states. On `Finished`
   * the installer has applied the build and we relaunch into the new version.
   * A rejection is classified into a download vs install failure state — both
   * leave the app blocked with a "TENTAR NOVAMENTE" action.
   */
  const runDownloadAndInstall = useCallback(
    async (handle: Update): Promise<void> => {
      setState("downloading");
      setProgressBytes(0);
      setProgressTotal(null);
      try {
        await handle.downloadAndInstall((event: DownloadEvent) => {
          if (event.event === "Started") {
            setProgressTotal(event.data.contentLength ?? null);
          } else if (event.event === "Progress") {
            setProgressBytes((b) => b + event.data.chunkLength);
          } else if (event.event === "Finished") {
            setState("installing");
          }
        });
        // install finished — relaunch into the fresh build. (The Finished
        // callback already flipped `installing`; this covers any path where the
        // promise resolves before the callback runs.)
        setState("installing");
        await relaunch();
      } catch (err) {
        console.error("[UpdateGate] downloadAndInstall() failed:", err);
        setState(classifyInstallError(err) === "install" ? "install-error" : "download-error");
      }
    },
    [],
  );

  // Only `up-to-date` releases the blocked children (the real <App/>): the
  // socket never opens, loadIdentity never runs, no activity/relay traffic
  // happens until the check confirms this is the newest build.
  if (state === "up-to-date") return <>{children}</>;

  // ---- Blocking screens (one action each, never a dismiss/ignore control) ----

  const onPrimary = (): void => {
    if (state === "available" && update) {
      void runDownloadAndInstall(update);
    } else if (state === "download-error" && update) {
      // A download failure keeps the same checked Update handle — retry the
      // download directly rather than re-running check() (the manifest is
      // already known-good; only the transfer failed).
      void runDownloadAndInstall(update);
    } else if (state === "install-error") {
      // An install failure may indicate a corrupt staged download — start
      // over from a fresh check + manifest fetch.
      void runCheck();
    } else if (state === "check-error") {
      void runCheck();
    }
  };

  return (
    <div className="window-main h-screen flex flex-col items-center justify-center px-6 text-center">
      <Shield />

      {state === "checking" && (
        <Screen
          title="Verificando atualizações…"
          subtitle="O Harbor está conferindo se esta é a versão mais recente."
          spinner
        />
      )}

      {state === "available" && (
        <Screen
          title="Atualização obrigatória"
          subtitle={
            <span>
              Versão instalada <b>{currentVersion}</b> · versão disponível{" "}
              <b>{newVersion}</b>
              <br />
              Nenhuma versão antiga pode continuar sendo usada. Atualize para
              continuar.
            </span>
          }
          action={{
            label: "ATUALIZAR AGORA",
            onClick: onPrimary,
          }}
        />
      )}

      {state === "downloading" && (
        <Screen
          title="Baixando atualização…"
          subtitle={
            progressTotal != null
              ? `${formatBytes(progressBytes)} / ${formatBytes(progressTotal)}`
              : formatBytes(progressBytes)
          }
          progress={progressTotal != null ? Math.min(100, (progressBytes / progressTotal) * 100) : null}
        />
      )}

      {state === "installing" && (
        <Screen
          title="Instalando…"
          subtitle="O Harbor vai reiniciar sozinho em instantes."
          spinner
        />
      )}

      {state === "download-error" && (
        <Screen
          title="Não foi possível baixar a atualização"
          subtitle="Verifique sua conexão e tente novamente. O app permanece bloqueado."
          action={{
            label: "TENTAR NOVAMENTE",
            onClick: onPrimary,
          }}
        />
      )}

      {state === "install-error" && (
        <Screen
          title="Não foi possível instalar a atualização"
          subtitle="O pacote baixado pode estar corrompido. Vamos verificar novamente."
          action={{
            label: "TENTAR NOVAMENTE",
            onClick: onPrimary,
          }}
        />
      )}

      {state === "check-error" && (
        <Screen
          title="Servidor de atualização indisponível"
          subtitle="Sem conexão ou o servidor está indisponível. O app só abre quando confirmamos que está atualizado."
          action={{
            label: "TENTAR NOVAMENTE",
            onClick: onPrimary,
          }}
        />
      )}
    </div>
  );
}

/** A small ocean-themed harbor/shield mark to anchor each blocking screen. */
function Shield(): ReactNode {
  return (
    <div
      className="mb-6 h-16 w-16 rounded-2xl flex items-center justify-center text-3xl"
      style={{
        background: "linear-gradient(135deg, var(--color-harbor-sea), var(--color-harbor-deep))",
        boxShadow: "0 8px 24px rgba(43, 108, 176, 0.28)",
      }}
      aria-hidden
    >
      ⚓
    </div>
  );
}

/** Spinner-less "thinking" indicator used by the checking + installing states. */
function Spinner(): ReactNode {
  return (
    <div className="mt-4 h-2 w-40 overflow-hidden rounded-full bg-harbor-line">
      <div className="reconnect-pulse h-full w-full rounded-full bg-harbor-sea" />
    </div>
  );
}

interface ScreenProps {
  title: string;
  subtitle?: ReactNode;
  action?: { label: string; onClick: () => void };
  spinner?: boolean;
  /** 0–100 progress for the download bar, or null for indeterminate. */
  progress?: number | null;
  "data-testid"?: string;
}

/** One blocking screen. Title + optional subtitle + optional single action,
 *  and either an indeterminate spinner or a determinate progress bar. */
function Screen({ title, subtitle, action, spinner, progress }: ScreenProps): ReactNode {
  return (
    <div className="flex max-w-sm flex-col items-center">
      <h1 className="text-2xl font-semibold" style={{ color: "var(--color-harbor-ink)" }}>
        {title}
      </h1>
      {subtitle != null && (
        <p className="mt-3 text-sm leading-relaxed" style={{ color: "var(--color-harbor-sec)" }}>
          {subtitle}
        </p>
      )}
      {progress != null ? (
        <div
          className="mt-5 h-2 w-64 overflow-hidden rounded-full bg-harbor-line"
          role="progressbar"
          aria-valuenow={Math.round(progress)}
          aria-valuemin={0}
          aria-valuemax={100}
        >
          <div
            className="h-full rounded-full bg-harbor-sea transition-[width] duration-150"
            style={{ width: `${Math.max(2, Math.min(100, progress))}%` }}
          />
        </div>
      ) : spinner ? (
        <Spinner />
      ) : null}
      {action && (
        <button
          type="button"
          onClick={action.onClick}
          className="mt-8 px-7 py-2.5 rounded-xl text-white font-medium shadow-sm transition-transform active:scale-[0.98]"
          style={{ background: "var(--color-harbor-deep)" }}
        >
          {action.label}
        </button>
      )}
    </div>
  );
}

/** Format a byte count for human-readable download progress. */
function formatBytes(n: number): string {
  if (!n || n < 0) return "0 B";
  const units = ["B", "KB", "MB", "GB"];
  let i = 0;
  let v = n;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i++;
  }
  return `${v.toFixed(v >= 10 || i === 0 ? 0 : 1)} ${units[i]}`;
}
