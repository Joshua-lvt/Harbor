/**
 * Sidebar — the dark, glassy left rail of the Home layout (≈240px).
 *
 * Holds: the Harbor wordmark + a small shark, a local-user block (avatar,
 * name, my own presence dot), and the vertical navigation. Real routes use the
 * existing hash routing (Início/Chat/Notificações/Atividade/Configurações). The
 * voice call has no nav entry anymore — it's always on and lives as the
 * `CallStrip` at the top of the main column, not a separate screen. The footer
 * reuses `MyActivityBadge` so the local "what I'm doing right now" stays
 * accurate without a second source.
 */
import { Home, MessageCircle, Gamepad2, Settings, Bell, Palette } from "lucide-react";
import { SharkMascot } from "../../assets/shark";
import { Avatar } from "../../components/Avatar";
import { MyActivityBadge } from "./ActivityBadge";
import type { Identity } from "../../lib/types";

type NavItem = {
  key: string;
  label: string;
  icon: typeof Home;
  route?: string;
};

const NAV: NavItem[] = [
  { key: "home", label: "Início", icon: Home, route: "#/home" },
  { key: "chat", label: "Chat", icon: MessageCircle, route: "#/chat" },
  // Feature 5: the quick-message send surface lives in its own tab now — the
  // presets no longer sit on Home. The receiver shows them as a giant overlay
  // (never the OS notification, never a chat bubble).
  { key: "notifications", label: "Notificações", icon: Bell, route: "#/notifications" },
  // Personalização: recolor the app (palettes per light/dark mode) + extras.
  { key: "personalization", label: "Personalização", icon: Palette, route: "#/personalization" },
  // Feature 3: Atividade is a real route (the chronological history screen),
  // not a disabled pip. The nav highlight uses the hash prefix.
  { key: "activity", label: "Atividade", icon: Gamepad2, route: "#/activity" },
  { key: "settings", label: "Configurações", icon: Settings, route: "#/settings" },
];

export function Sidebar({
  identity,
  connected,
  currentHash,
  partnerActive,
  myActivity,
  shareActivity,
}: {
  identity: Identity;
  connected: boolean;
  currentHash: string;
  partnerActive: boolean;
  myActivity: string | null;
  shareActivity: boolean;
}) {
  const myName = identity.my_name?.trim() || "Você";
  const selfDot = connected ? "dot-online" : "dot-offline";

  return (
    <aside className="flex h-full w-60 shrink-0 flex-col bg-harbor-sidebar/95 text-white/90 backdrop-blur-md">
      {/* Brand */}
      <div className="flex items-center gap-2 px-5 py-4">
        <SharkMascot className="harbor-mascot h-8 w-8" />
        <span className="text-lg font-semibold tracking-tight text-white">Harbor</span>
      </div>

      {/* Local user block */}
      <div className="mx-3 mb-2 flex items-center gap-3 rounded-2xl bg-white/5 px-3 py-3">
        <div className="relative shrink-0">
          <Avatar src={identity.my_avatar ?? null} alt={myName} size={44} />
          <span
            className={`absolute bottom-0 right-0 h-3 w-3 rounded-full ${selfDot} ring-2 ring-harbor-sidebar`}
          />
        </div>
        <div className="min-w-0">
          <p className="truncate text-sm font-medium text-white">{myName}</p>
          <p className="text-xs text-white/55">{connected ? "Online" : "Offline"}</p>
        </div>
      </div>

      {/* Navigation */}
      <nav className="mt-1 flex flex-1 flex-col gap-1 px-3">
        {NAV.map((item) => {
          const Icon = item.icon;
          // Every NAV item is a route now (Atividade + Notificações included).
          // Match the full route so '#/not' doesn't blur '#/notifications' into
          // '#/notif-overlay' (different windows, but the highlight logic owns
          // only the main window's hash here).
          const active = !!item.route && currentHash === item.route;
          // Keep the live activity pip on the Atividade item so the partner's
          // "currently using …" glance stays visible even though it's a route.
          const stateLit = item.key === "activity" ? partnerActive : false;

          return (
            <button
              key={item.key}
              onClick={() => item.route && (window.location.hash = item.route)}
              className={[
                "flex items-center gap-3 rounded-xl px-3 py-2.5 text-sm transition",
                active
                  ? "bg-harbor-sea/25 text-white"
                  : stateLit
                    ? "text-white/80 hover:bg-white/10"
                    : "text-white/75 hover:bg-white/10 hover:text-white",
              ].join(" ")}
            >
              <Icon className="h-5 w-5 shrink-0" />
              <span className="flex-1 text-left">{item.label}</span>
              {/* Live state pip — lights when the partner is actively using an app. */}
              {item.key === "activity" && (
                <span className={`h-2 w-2 rounded-full ${partnerActive ? "bg-harbor-ice" : "bg-white/25"}`} />
              )}
            </button>
          );
        })}
      </nav>

      {/* Footer: my own live activity (reuses the existing badge). */}
      <div className="border-t border-white/10 px-3 py-3">
        <MyActivityBadge exe={myActivity} sharing={shareActivity} />
      </div>
    </aside>
  );
}

export default Sidebar;
