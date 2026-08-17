/**
 * App theme application (Light / Dark / System).
 *
 * Dark mode is class-based (not media-based): `applyTheme` toggles the `.dark`
 * class on <html>, which activates the `dark:` variant and the `.dark` token
 * overrides declared in style.css. Utilities like `bg-harbor-bg` / `text-ink`
 * then resolve to their dark values wherever `.dark` is an ancestor — so most
 * of the app flips automatically without per-component `dark:` classes.
 *
 * "system" resolves to the OS preference via `prefers-color-scheme` AND listens
 * for changes, so flipping the OS theme re-themes the app live (no restart).
 *
 * `applyTheme` returns a cleanup that removes the OS listener — callers (the
 * App effect, re-run whenever `settings.theme` changes) must invoke it so an
 * old "system" listener doesn't keep shadowing a newly-chosen fixed theme.
 */
export type ThemeOption = "light" | "dark" | "system";

const DARK_QUERY = "(prefers-color-scheme: dark)";

/** True when the OS currently prefers dark. Safe to call before CSS/DOM is ready
 *  (WebView2 has matchMedia at module load); used by main.tsx to paint the
 *  initial class before React mounts and avoid a light flash on a dark OS. */
export function systemPrefersDark(): boolean {
  return typeof window !== "undefined" && "matchMedia" in window && window.matchMedia(DARK_QUERY).matches;
}

function setDark(on: boolean): void {
  document.documentElement.classList.toggle("dark", on);
}

function resolvedDark(theme: ThemeOption): boolean {
  return theme === "dark" || (theme === "system" && systemPrefersDark());
}

/** Apply a theme and, for "system", start watching the OS preference. Returns a
 *  cleanup that removes the OS listener (call it before re-applying a new theme
 *  or on unmount). For fixed themes the cleanup is a no-op. */
export function applyTheme(theme: ThemeOption): () => void {
  setDark(resolvedDark(theme));
  if (theme !== "system") return () => {};
  const mq = window.matchMedia(DARK_QUERY);
  const handler = () => setDark(mq.matches);
  mq.addEventListener("change", handler);
  return () => mq.removeEventListener("change", handler);
}

/** Boot-time helper: paint the theme before React mounts (avoids a flash).
 *  Same as `applyTheme`; the name just signals intent at call sites. */
export const initTheme = applyTheme;
