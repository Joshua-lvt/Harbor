/**
 * ActivitiesScreen — the real chronological activity history (Feature 3).
 *
 * Loads the persisted `activity_history` rows for the partner (migrated v3 in
 * lib.rs, populated by `ActivityTracker` in App.tsx), groups them by period
 * (Hoje / Ontem / Esta semana / Antes), and renders a chronological list:
 * app icon (Feature 4 via `getAppIcon`, with a `GeneratedAppIcon` fallback) +
 * friendly name + event label + duration + time.
 *
 * Routed at `#/activity` (App.tsx getRoute()); the Sidebar nav item flips from
 * a disabled pip to this real route. Owns no socket — it reads what App's
 * tracker already persisted on its own mount.
 */
import { useEffect, useState } from "react";
import { loadActivityHistory, type ActivityEvent } from "../../lib/activityHistory";
import { friendlyName } from "../../lib/appNames";
import { getAppIcon, GeneratedAppIcon } from "../../lib/appIconCache";
import type { Identity } from "../../lib/types";

type Group = { label: string; items: ActivityEvent[] };

/** Bucket each row's start time into Hoje / Ontem / Esta semana / Antes. */
function groupByPeriod(events: ActivityEvent[]): Group[] {
  const now = new Date();
  const startOfToday = new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime();
  const startOfYesterday = startOfToday - 86400_000;
  const startOfWeek = startOfToday - 6 * 86400_000; // last 7 days incl. today

  const groups: Record<string, ActivityEvent[]> = {
    "Hoje": [],
    "Ontem": [],
    "Esta semana": [],
    "Antes": [],
  };
  for (const e of events) {
    if (e.started_at >= startOfToday) groups["Hoje"].push(e);
    else if (e.started_at >= startOfYesterday) groups["Ontem"].push(e);
    else if (e.started_at >= startOfWeek) groups["Esta semana"].push(e);
    else groups["Antes"].push(e);
  }
  const order = ["Hoje", "Ontem", "Esta semana", "Antes"];
  return order.map((label) => ({ label, items: groups[label] })).filter((g) => g.items.length);
}

/** "Usou por 35 min" / "Usou por 2 h 14 min" from duration_sec. */
function durationLabel(sec: number | null): string {
  if (sec == null) return "";
  if (sec < 60) return `Usou por ${sec} s`;
  const m = Math.floor(sec / 60);
  if (m < 60) return `Usou por ${m} min`;
  const h = Math.floor(m / 60);
  const rem = m % 60;
  return rem ? `Usou por ${h} h ${rem} min` : `Usou por ${h} h`;
}

/** Event label per row: "Abriu X" / "Fechou X" / "Trocou para X". */
function eventLabel(e: ActivityEvent): string {
  const name = e.exe ? friendlyName(e.exe) : "—";
  switch (e.event) {
    case "open":
      return `Abriu ${name}`;
    case "switch":
      return `Trocou para ${name}`;
    case "close":
      return `Fechou ${name}`;
    default:
      return e.exe ? name : "Ocioso";
  }
}

function timeLabel(ms: number): string {
  const d = new Date(ms);
  const hh = d.getHours().toString().padStart(2, "0");
  const mm = d.getMinutes().toString().padStart(2, "0");
  return `${hh}:${mm}`;
}

function Row({ e }: { e: ActivityEvent }) {
  const [icon, setIcon] = useState<string | null | undefined>(undefined);
  useEffect(() => {
    // Look up the cached partner icon by exe name (receiver side: the icon was
    // pushed once via `activity_icon` and cached locally). undefined = not yet
    // resolved; null = no icon cached → generated fallback.
    let cancelled = false;
    void getAppIcon("", e.exe ?? "").then((i) => {
      if (!cancelled) setIcon(i ?? null);
    });
    return () => {
      cancelled = true;
    };
  }, [e.exe]);

  return (
    <li className="flex items-center gap-3 rounded-xl bg-harbor-surface/60 px-3 py-2.5">
      {icon ? (
        <img
          src={icon}
          alt=""
          className="h-8 w-8 shrink-0 rounded-md object-contain"
          draggable={false}
        />
      ) : (
        <GeneratedAppIcon exe={e.exe ?? ""} size={32} className="shrink-0" />
      )}
      <div className="min-w-0 flex-1">
        <p className="truncate text-sm font-medium text-harbor-ink">{eventLabel(e)}</p>
        {e.duration_sec != null && (
          <p className="text-xs text-harbor-ink/55">{durationLabel(e.duration_sec)}</p>
        )}
      </div>
      <span className="shrink-0 text-xs text-harbor-ink/45">{timeLabel(e.started_at)}</span>
    </li>
  );
}

export default function ActivitiesScreen({
  identity,
  back,
}: {
  identity: Identity;
  back: () => void;
}) {
  const partnerId = identity.partner_id!;
  const [events, setEvents] = useState<ActivityEvent[] | null>(null);

  // Load history on mount. null = loading; [] = empty (still fine).
  useEffect(() => {
    let cancelled = false;
    void loadActivityHistory(partnerId).then((rows) => {
      if (!cancelled) setEvents(rows);
    });
    return () => {
      cancelled = true;
    };
  }, [partnerId]);

  const groups = events ? groupByPeriod(events) : [];

  return (
    <div className="window-main h-screen flex flex-col">
      <div className="flex items-center gap-3 px-4 py-3 border-b border-harbor-line bg-harbor-surface/60 backdrop-blur">
        <button onClick={back} className="text-sm text-harbor-deep">← Voltar</button>
        <span className="font-semibold text-harbor-deep">Atividade</span>
      </div>

      <div className="flex-1 overflow-y-auto p-4">
        {events === null ? (
          <p className="text-sm text-harbor-ink/50">Carregando…</p>
        ) : groups.length === 0 ? (
          <div className="flex flex-col items-center justify-center gap-2 py-16 text-center">
            <span className="text-3xl">📭</span>
            <p className="text-sm text-harbor-ink/55">
              Nenhuma atividade registrada ainda.
            </p>
            <p className="text-xs text-harbor-ink/40">
              O histórico começa a partir do primeiro app que o parceiro abrir enquanto
              vocês estão conectados.
            </p>
          </div>
        ) : (
          <div className="flex flex-col gap-6">
            {groups.map((g) => (
              <section key={g.label} className="flex flex-col gap-2">
                <h2 className="px-1 text-xs font-semibold uppercase tracking-wide text-harbor-deep/60">
                  {g.label}
                </h2>
                <ul className="flex flex-col gap-2">
                  {g.items.map((e) => (
                    <Row key={e.id} e={e} />
                  ))}
                </ul>
              </section>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
