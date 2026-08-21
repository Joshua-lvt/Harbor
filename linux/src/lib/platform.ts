/**
 * platform — lightweight OS detection for the frontend.
 *
 * Used to make user-facing strings platform-correct (e.g. the mic-blocked
 * message must say "Windows" on Windows and "Linux" on Linux — the old code
 * hardcoded "O Windows está bloqueando..." which made no sense on Linux).
 * Detection is via the WebView user agent: WebView2 (Windows) reports
 * "Windows NT", WebKitGTK (Linux) reports "Linux". No extra dependency.
 */

export function isWindows(): boolean {
  return /windows/i.test(navigator.userAgent);
}

export function isLinux(): boolean {
  return /linux/i.test(navigator.userAgent);
}

/** Human-readable platform name for error strings. */
export function platformName(): string {
  if (isWindows()) return "Windows";
  if (isLinux()) return "Linux";
  return "sistema";
}
