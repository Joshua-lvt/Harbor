/**
 * PersonalizationScreen — the Personalização tab (#/personalization).
 *
 * Lets the user repaint Harbor: pick a theme (Claro/Escuro/Sistema, moved here
 * from Configurações), swatch a preset palette, or hand-edit the 6 color tokens
 * for the *current* resolved mode (light and dark keep separate palettes, so a
 * Sistema user edits each one as they live-flip). Plus two extras: the
 * OceanBackground style (Ondas/Sólido) and whether the harbor shark mascot shows
 * in the main-app chrome.
 *
 * Every control applies IMMEDIATELY (no Salvar) — persist + lift to App state on
 * each change, mirroring the live-apply theme pattern already used in Settings.
 * The color inputs are debounced 120ms to a save so dragging a native picker
 * doesn't hammer the store. Owns no socket (mirrors ActivitiesScreen).
 */
import { useMemo, useRef, useState, useEffect } from "react";
import { applyTheme, type ThemeOption } from "../../lib/theme";
import { saveSettings } from "../../lib/identity";
import {
  type AccentPreset,
  type BackgroundStyle,
  type Customization,
  type ModePalette,
  PRESET_PALETTES,
  DEFAULT_CUSTOMIZATION,
  OCEAN_LIGHT,
  OCEAN_DARK,
  presetSwatch,
} from "../../lib/customization";
import type { Settings } from "../../lib/types";

const ACCENTS: { id: Exclude<AccentPreset, "custom">; label: string }[] = [
  { id: "ocean", label: "Oceano" },
  { id: "lavender", label: "Lavanda" },
  { id: "rose", label: "Rosa" },
  { id: "sunset", label: "Pôr do sol" },
  { id: "forest", label: "Floresta" },
  { id: "grape", label: "Uva" },
];

function resolvedDark(theme: ThemeOption): boolean {
  return (
    theme === "dark" ||
    (theme === "system" &&
      window.matchMedia("(prefers-color-scheme: dark)").matches)
  );
}

/** Which palette fields a color input edits, with UI labels + which CSS var. */
const FIELDS: { key: keyof ModePalette; label: string; hint: string }[] = [
  { key: "sea", label: "Destaque", hint: "Botões, links e estados ativos" },
  { key: "deep", label: "Profundo", hint: "Texto de botões e títulos" },
  { key: "ink", label: "Texto", hint: "Cor principal do texto" },
  { key: "bg", label: "Fundo", hint: "Topo do gradiente de fundo" },
  { key: "bg2", label: "Fundo 2", hint: "Base do gradiente de fundo" },
  { key: "sidebar", label: "Barra lateral", hint: "O painel escuro à esquerda" },
];

export default function PersonalizationScreen({
  settings,
  onSettings,
  back,
}: {
  settings: Settings;
  onSettings: (s: Settings) => void;
  back: () => void;
}) {
  // The live customization always reflects App's settings (lifted state); we keep
  // a local draft for the debounced color inputs so dragging a native picker
  // stays smooth and only persists on settle.
  const c: Customization = settings.customization ?? DEFAULT_CUSTOMIZATION;
  const [draftMode, setDraftMode] = useState<ModePalette | null>(null);
  const saveTimer = useRef<number | null>(null);

  const dark = useMemo(() => resolvedDark(settings.theme), [settings.theme]);

  /** Persist + lift a new customization (live-applied by App's effect). */
  async function commit(next: Customization) {
    const s: Settings = { ...settings, customization: next };
    onSettings(s);
    await saveSettings(s);
  }

  /** Apply a preset to BOTH modes (so a Sistema flip + mode toggle show the
   *  matching preset colors), set `accent`, commit immediately. */
  async function pickPreset(id: Exclude<AccentPreset, "custom">) {
    const { light, dark: darkPal } = PRESET_PALETTES[id];
    await commit({ ...c, accent: id, light: { ...light }, dark: { ...darkPal } });
  }

  /** A per-token color edit targets ONLY the currently-resolved mode so the user
   *  sees live changes; the other mode is left untouched. Marks `accent: custom`.
   *  Persisted on a 120ms debounce so a native color picker drag doesn't spam. */
  function editToken(key: keyof ModePalette, value: string) {
    const base = draftMode ?? (dark ? { ...c.dark } : { ...c.light });
    base[key] = value;
    setDraftMode(base);
    if (saveTimer.current) clearTimeout(saveTimer.current);
    saveTimer.current = window.setTimeout(() => {
      const next: Customization = {
        ...c,
        accent: "custom",
        ...(dark ? { dark: { ...base } } : { light: { ...base } }),
      };
      void commit(next);
      setDraftMode(null);
    }, 120);
  }

  /** Restore the ocean defaults into the CURRENT mode only (`accent: ocean`). The
   *  other mode keeps whatever it had — Redefinir is "reset this mode," not
   *  "reset everything." */
  async function resetMode() {
    const ocean = dark ? OCEAN_DARK : OCEAN_LIGHT;
    await commit({
      ...c,
      accent: "ocean",
      ...(dark ? { dark: { ...ocean } } : { light: { ...ocean } }),
    });
  }

  /** Apply a theme choice immediately (same live pattern as the old Settings
   *  Aparência section), persist it, and lift to App state. App's applyTheme +
   *  customization effects re-run on `settings.theme`. */
  async function setTheme(theme: ThemeOption) {
    applyTheme(theme);
    const next: Settings = { ...settings, theme };
    onSettings(next);
    await saveSettings(next);
  }

  async function setBackground(background: BackgroundStyle) {
    await commit({ ...c, background });
  }
  async function setMascot(showMascot: boolean) {
    await commit({ ...c, showMascot });
  }

  // The displayed palette = local draft (during a drag) else the live mode palette.
  const modePalette: ModePalette = draftMode ?? (dark ? c.dark : c.light);
  const modeLabel = dark ? "Escuro" : "Claro";

  useEffect(
    () => () => {
      if (saveTimer.current) clearTimeout(saveTimer.current);
    },
    [],
  );

  return (
    <div className="window-main h-screen flex flex-col">
      <div className="flex items-center gap-3 px-4 py-3 border-b border-harbor-line bg-harbor-surface/60 backdrop-blur">
        <button onClick={back} className="text-sm text-harbor-deep">
          ← Voltar
        </button>
        <span className="font-semibold text-harbor-deep">Personalização</span>
      </div>

      <div className="flex-1 overflow-y-auto p-4">
        <div className="mx-auto flex max-w-xl flex-col gap-6">
          {/* Tema — moved here from Configurações (Aparência). */}
          <section className="flex flex-col gap-2">
            <p className="text-xs font-medium text-harbor-deep/70">Tema</p>
            <div className="flex items-center gap-1 rounded-xl bg-harbor-surface/70 p-1 self-start">
              {(
                [
                  ["light", "Claro"],
                  ["dark", "Escuro"],
                  ["system", "Sistema"],
                ] as const
              ).map(([value, label]) => {
                const active = settings.theme === value;
                return (
                  <button
                    key={value}
                    type="button"
                    onClick={() => void setTheme(value)}
                    className={[
                      "rounded-lg px-4 py-1.5 text-sm font-medium transition",
                      active
                        ? "bg-harbor-deep text-white shadow-sm"
                        : "text-harbor-ink/70 hover:text-harbor-ink hover:bg-harbor-surface-strong",
                    ].join(" ")}
                  >
                    {label}
                  </button>
                );
              })}
            </div>
            <p className="text-[11px] text-harbor-ink/50">
              "Sistema" segue o tema do Windows — muda sozinho quando você alterna.
              Cada tema (claro/escuro) tem sua própria paleta abaixo.
            </p>
          </section>

          {/* Cor de destaque — presets. */}
          <section className="flex flex-col gap-2">
            <p className="text-xs font-medium text-harbor-deep/70">
              Cor de destaque
            </p>
            <div className="flex flex-wrap gap-2">
              {ACCENTS.map((a) => {
                const active = c.accent === a.id;
                return (
                  <button
                    key={a.id}
                    type="button"
                    onClick={() => void pickPreset(a.id)}
                    title={a.label}
                    className={[
                      "flex items-center gap-2 rounded-xl border px-3 py-2 text-sm transition",
                      active
                        ? "border-harbor-sea bg-harbor-surface-strong text-harbor-ink shadow-sm"
                        : "border-harbor-line bg-harbor-surface/70 text-harbor-ink/80 hover:bg-harbor-surface-strong",
                    ].join(" ")}
                  >
                    <span
                      className="h-5 w-5 rounded-full border border-black/10"
                      style={{ backgroundColor: presetSwatch(a.id) }}
                    />
                    {a.label}
                  </button>
                );
              })}
            </div>
          </section>

          {/* Paleta personalizada — per-token edits for the current mode. */}
          <section className="flex flex-col gap-2">
            <div className="flex items-center justify-between">
              <p className="text-xs font-medium text-harbor-deep/70">
                Paleta personalizada
              </p>
              <span className="rounded-full bg-harbor-sky/40 px-2 py-0.5 text-[10px] font-medium text-harbor-deep">
                Editando: tema {modeLabel}
              </span>
            </div>
            <div className="grid grid-cols-1 gap-2 sm:grid-cols-2">
              {FIELDS.map((f) => (
                <label
                  key={f.key}
                  className="flex items-center gap-3 rounded-xl bg-harbor-surface/70 px-3 py-2"
                >
                  <input
                    type="color"
                    value={modePalette[f.key]}
                    onChange={(e) => editToken(f.key, e.target.value)}
                    className="h-8 w-8 shrink-0 cursor-pointer rounded-md border border-harbor-line bg-transparent"
                    aria-label={f.label}
                  />
                  <div className="min-w-0 flex-1">
                    <p className="text-sm font-medium text-harbor-ink">
                      {f.label}
                    </p>
                    <p className="text-[11px] text-harbor-ink/50">{f.hint}</p>
                  </div>
                </label>
              ))}
            </div>
            <button
              type="button"
              onClick={() => void resetMode()}
              className="self-start rounded-lg border border-harbor-line bg-harbor-surface/70 px-3 py-1.5 text-xs font-medium text-harbor-ink/80 transition hover:bg-harbor-surface-strong"
            >
              Redefinir tema {modeLabel.toLowerCase()}
            </button>
            <p className="text-[11px] text-harbor-ink/50">
              As cores se aplicam na hora. Editar muda só o tema {modeLabel.toLowerCase()} —
              troque o tema acima para ajustar o outro.
            </p>
          </section>

          {/* Extras — background style + mascot. */}
          <section className="flex flex-col gap-3">
            <p className="text-xs font-medium text-harbor-deep/70">Extras</p>

            <div className="flex flex-col gap-2">
              <span className="text-sm text-harbor-ink">Fundo</span>
              <div className="flex items-center gap-1 rounded-xl bg-harbor-surface/70 p-1 self-start">
                {(
                  [
                    ["waves", "Ondas"],
                    ["solid", "Sólido"],
                  ] as const
                ).map(([value, label]) => {
                  const active = c.background === value;
                  return (
                    <button
                      key={value}
                      type="button"
                      onClick={() => void setBackground(value)}
                      className={[
                        "rounded-lg px-4 py-1.5 text-sm font-medium transition",
                        active
                          ? "bg-harbor-deep text-white shadow-sm"
                          : "text-harbor-ink/70 hover:text-harbor-ink hover:bg-harbor-surface-strong",
                      ].join(" ")}
                    >
                      {label}
                    </button>
                  );
                })}
              </div>
            </div>

            <label className="flex items-center justify-between rounded-xl bg-harbor-surface/70 px-3 py-2">
              <div>
                <p className="text-sm text-harbor-ink">Mascote tubarão</p>
                <p className="text-[11px] text-harbor-ink/50">
                  Mostra o tubarão na barra lateral e no avatar padrão.
                </p>
              </div>
              <input
                type="checkbox"
                checked={c.showMascot}
                onChange={(e) => void setMascot(e.target.checked)}
                className="h-4 w-4 accent-harbor-deep"
              />
            </label>
          </section>
        </div>
      </div>
    </div>
  );
}
