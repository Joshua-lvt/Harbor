/**
 * Foreground-app → friendly-name mapping + game detection (Discord-style).
 *
 * The relay forwards the raw lowercased EXE name (e.g. "code.exe"); the
 * RECEIVER maps it to a friendly display name and decides whether it's a
 * game. Privacy: only the process name is shared — never the window title,
 * never screen content.
 */

/** Raw exe (lowercased) → friendly display name. */
export const EXE_TO_NAME: Record<string, string> = {
  "chrome.exe": "Google Chrome",
  "msedge.exe": "Microsoft Edge",
  "firefox.exe": "Firefox",
  "brave.exe": "Brave",
  "code.exe": "Visual Studio Code",
  "code - insiders.exe": "Visual Studio Code",
  "cursor.exe": "Cursor",
  "devenv.exe": "Visual Studio",
  "idea64.exe": "IntelliJ IDEA",
  "webstorm64.exe": "WebStorm",
  "windowsterminal.exe": "Terminal",
  "cmd.exe": "Terminal",
  "powershell.exe": "PowerShell",
  "explorer.exe": "Explorador de Arquivos",
  "spotify.exe": "Spotify",
  "discord.exe": "Discord",
  "telegram.exe": "Telegram",
  "whatsapp.exe": "WhatsApp",
  "vlc.exe": "VLC",
  "obs64.exe": "OBS Studio",
  "notepad.exe": "Bloco de Notas",
  "winword.exe": "Word",
  "excel.exe": "Excel",
  "powerpnt.exe": "PowerPoint",
  // Games (also detected by detectGame):
  "valorant.exe": "Valorant",
  "csgo.exe": "CS:GO",
  "cs2.exe": "Counter-Strike 2",
  "league of legends.exe": "League of Legends",
  "leagueclient.exe": "League of Legends",
  "dota2.exe": "Dota 2",
  "fortnite.exe": "Fortnite",
  "genshinimpact.exe": "Genshin Impact",
  "javaw.exe": "Minecraft",
  "overwatch.exe": "Overwatch",
  "wow.exe": "World of Warcraft",
};

/** Exe names that count as "playing a game". */
export const GAME_EXES = new Set<string>([
  "valorant.exe",
  "csgo.exe",
  "cs2.exe",
  "league of legends.exe",
  "leagueclient.exe",
  "dota2.exe",
  "fortnite.exe",
  "genshinimpact.exe",
  "javaw.exe",
  "overwatch.exe",
  "wow.exe",
  "steam.exe",
  "riotclientservices.exe",
  "epicgameslauncher.exe",
  "minecraft.exe",
  "lol.exe",
  "valorant.exe",
]);

/** Friendly display name for a lowercased exe, falling back to a capitalized basename. */
export function friendlyName(exe: string): string {
  const key = exe.toLowerCase();
  return (
    EXE_TO_NAME[key] ??
    key.replace(/\.exe$/i, "").replace(/\b\w/g, (c) => c.toUpperCase())
  );
}

/** If the exe is a known game, return its friendly name; else null. */
export function detectGame(exe: string | null): string | null {
  if (!exe) return null;
  const key = exe.toLowerCase();
  return GAME_EXES.has(key) ? friendlyName(key) : null;
}
