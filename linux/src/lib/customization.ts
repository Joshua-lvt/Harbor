/**
 * Harbor personalization — user-recolorable palette, persisted in the Settings
 * store and applied live to `:root` via inline `--color-harbor-*` overrides.
 *
 * The palette is stored PER MODE (light + dark) so the dark path is respected:
 * light and dark keep separate custom palettes, and "Sistema" re-applies the
 * right one when the OS flips (App.tsx re-runs `applyCustomization` on the OS
 * change listener). `applyCustomization(c, dark)` writes only the resolved
 * mode's 6 tokens to `document.documentElement.style` (inline, which outranks
 * both the `@theme :root` declarations and the `.dark` overrides in style.css),
 * so every Harbor utility — including opacity variants like `bg-harbor-sea/15`,
 * which Tailwind v4 compiles to `color-mix(in oklab, var(--color-harbor-sea) 15%,
 * transparent)` — restyles automatically. No per-utility edits anywhere.
 *
 * Tokens NOT customized here (sky/ice/mist/surface/...) keep their `@theme` /
 * `.dark` defaults — the 6 here (sea/deep/ink/bg/bg2/sidebar) are the ones that
 * visually define the mood, INCLUDING the left sidebar rail (a dark nav surface
 * in both themes, so it has its own token rather than reusing deep/bg2), and
 * exposing only them keeps the tab legible and hard to make unreadable.
 * Fixed-hue elements (presence dots, the shark SVG's literal hex) intentionally
 * do NOT follow the custom palette.
 */
import type { Settings } from "./types";

export type AccentPreset =
  | "ocean"
  | "lavender"
  | "rose"
  | "sunset"
  | "forest"
  | "grape"
  | "custom";
export type BackgroundStyle = "waves" | "solid";

export interface ModePalette {
  /** --color-harbor-sea  — primary/accent (links, buttons, active states). */
  sea: string;
  /** --color-harbor-deep — deep accent (button labels, headings, icons). */
  deep: string;
  /** --color-harbor-ink  — main text color. */
  ink: string;
  /** --color-harbor-bg   — main app background (top of the ocean gradient). */
  bg: string;
  /** --color-harbor-bg2  — secondary background (bottom of the gradient). */
  bg2: string;
  /** --color-harbor-sidebar — the dark left nav rail. Distinct from deep/bg2
   *  because it's a dark surface in BOTH themes (the sidebar stays dark even in
   *  light mode), so a preset recolors it to match the accent family rather than
   *  reading as a fixed navy strip. Seeded with ocean's `#1e3a5f` everywhere so
   *  the app looks identical until a preset is chosen. */
  sidebar: string;
}

export interface Customization {
  /** Which preset is active; "custom" once a per-token color is hand-edited. */
  accent: AccentPreset;
  /** Palette applied when the resolved theme is light. */
  light: ModePalette;
  /** Palette applied when the resolved theme is dark. */
  dark: ModePalette;
  /** OceanBackground decorative bubbles/waves (waves) vs. flat solid (solid). */
  background: BackgroundStyle;
  /** Show the harbor shark mascot in main-app chrome (Sidebar + Avatar fallback
   *  + Chat empty state). In-feature branding (Pairing, Settings footer) is
   *  unaffected — this gates the persistent chrome, not marketing surfaces. */
  showMascot: boolean;
}

/** The 6 custom tokens → their `:root` CSS custom property names. */
const TOKEN_KEYS = ["sea", "deep", "ink", "bg", "bg2", "sidebar"] as const;
const TOKEN_VAR: Record<(typeof TOKEN_KEYS)[number], string> = {
  sea: "--color-harbor-sea",
  deep: "--color-harbor-deep",
  ink: "--color-harbor-ink",
  bg: "--color-harbor-bg",
  bg2: "--color-harbor-bg2",
  sidebar: "--color-harbor-sidebar",
};

/** The built-in ocean defaults, copied verbatim from style.css `@theme` (light)
 *  + `.dark` (dark) so the app renders identically until the user picks a preset
 *  or edits a color. Keeping them here (not re-derived from the CSS) means
 *  "Redefinir" can restore exact defaults without parsing the stylesheet. */
export const OCEAN_LIGHT: ModePalette = {
  sea: "#63b3ed",
  deep: "#2b6cb0",
  ink: "#1a365d",
  bg: "#f8fbff",
  bg2: "#eef4ff",
  sidebar: "#1e3a5f",
};
export const OCEAN_DARK: ModePalette = {
  sea: "#63b3ed",
  deep: "#2b6cb0",
  ink: "#e6edf6",
  bg: "#0b1220",
  bg2: "#0e1626",
  sidebar: "#1e3a5f",
};

export const DEFAULT_CUSTOMIZATION: Customization = {
  accent: "ocean",
  light: { ...OCEAN_LIGHT },
  dark: { ...OCEAN_DARK },
  background: "waves",
  showMascot: true,
};

/** Named preset palettes — `{ light, dark }` pairs a user can swatch to. Picking
 *  a preset writes BOTH its light + dark palettes into `customization.light/dark`
 *  so the mode toggle re-applies the matching preset colors. */
export const PRESET_PALETTES: Record<Exclude<AccentPreset, "custom">, { light: ModePalette; dark: ModePalette }> = {
  ocean: { light: OCEAN_LIGHT, dark: OCEAN_DARK },
  lavender: {
    light: { sea: "#a78bfa", deep: "#6d28d9", ink: "#3b1d6e", bg: "#faf8ff", bg2: "#f0e9ff", sidebar: "#2a1247" },
    dark: { sea: "#a78bfa", deep: "#8b5cf6", ink: "#e8e2ff", bg: "#0f0a1f", bg2: "#140e2b", sidebar: "#1f0f38" },
  },
  rose: {
    light: { sea: "#f687b3", deep: "#be185d", ink: "#5b1233", bg: "#fff6f9", bg2: "#ffe6f0", sidebar: "#3a0d22" },
    dark: { sea: "#f687b3", deep: "#ec4899", ink: "#ffe0ec", bg: "#1e0f17", bg2: "#251019", sidebar: "#300a1c" },
  },
  sunset: {
    light: { sea: "#fb923c", deep: "#c2410c", ink: "#5c2110", bg: "#fff9f5", bg2: "#ffecdf", sidebar: "#3a1408" },
    dark: { sea: "#fb923c", deep: "#ea580c", ink: "#ffe8d6", bg: "#1d100a", bg2: "#22140b", sidebar: "#2c0f06" },
  },
  forest: {
    light: { sea: "#34d399", deep: "#047857", ink: "#0c3d2e", bg: "#f6fdfb", bg2: "#e6fff5", sidebar: "#082818" },
    dark: { sea: "#34d399", deep: "#10b981", ink: "#d8fff0", bg: "#0a1d16", bg2: "#0b241b", sidebar: "#061d12" },
  },
  grape: {
    light: { sea: "#c084fc", deep: "#7e22ce", ink: "#3d1b5c", bg: "#fbf8ff", bg2: "#f3e8ff", sidebar: "#2a1142" },
    dark: { sea: "#c084fc", deep: "#a855f7", ink: "#f0e6ff", bg: "#150a24", bg2: "#1a0e2e", sidebar: "#200a35" },
  },
};

/** The swatch color for each preset (the `sea` hue of the LIGHT palette, since
 *  that's the dominant accent a swatch communicates). */
export function presetSwatch(p: AccentPreset): string {
  if (p === "custom") return "#64748b"; // neutral — never actually shown as active
  return PRESET_PALETTES[p].light.sea;
}

/** Apply a customization to the current document. Writes the resolved mode's 5
 *  tokens as inline CSS vars on `<html>` (outranks `:root` + `.dark` rules),
 *  toggles the `bg-solid` class (hides OceanBackground decorations when "solid"),
 *  and the `hide-mascot` class (gates `.harbor-mascot` elements). Idempotent. */
export function applyCustomization(c: Customization, dark: boolean): void {
  const pal = dark ? c.dark : c.light;
  const root = document.documentElement;
  for (const key of TOKEN_KEYS) {
    root.style.setProperty(TOKEN_VAR[key], pal[key]);
  }
  root.classList.toggle("bg-solid", c.background === "solid");
  root.classList.toggle("hide-mascot", !c.showMascot);
}

/** Remove the inline token overrides so the CSS `@theme` / `.dark` defaults show
 *  again, and clear the two gating classes. Used by the editing screen when a
 *  preset restores exact defaults (so nothing lingers inline). */
export function clearCustomization(): void {
  const root = document.documentElement;
  for (const key of TOKEN_KEYS) root.style.removeProperty(TOKEN_VAR[key]);
  root.classList.remove("bg-solid", "hide-mascot");
}

/** Resolve the customization field from a (possibly migration-era) Settings
 *  object — `DEFAULT_CUSTOMIZATION` merged over a partial stored value, so an
 *  old `settings.json` with no `customization` key inertly gets the ocean
 *  defaults. Lives here so both the load-merge and the screen can call it. */
export function resolveCustomization(c: Partial<Customization> | undefined): Customization {
  if (!c) return { ...DEFAULT_CUSTOMIZATION };
  return {
    accent: c.accent ?? DEFAULT_CUSTOMIZATION.accent,
    light: { ...DEFAULT_CUSTOMIZATION.light, ...(c.light ?? {}) },
    dark: { ...DEFAULT_CUSTOMIZATION.dark, ...(c.dark ?? {}) },
    background: c.background ?? DEFAULT_CUSTOMIZATION.background,
    showMascot: c.showMascot ?? DEFAULT_CUSTOMIZATION.showMascot,
  };
}

/** Convenience for App wiring: resolved customization from the live Settings. */
export function customizationOf(settings: Settings): Customization {
  return resolveCustomization(settings.customization);
}
