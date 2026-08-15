/**
 * A centered date chip between message groups when the calendar day changes.
 * "Hoje" / "Ontem" / "5 de agosto" (pt-BR).
 */
function dayLabel(ts: number): string {
  const d = new Date(ts);
  const now = new Date();
  const sameDay = (a: Date, b: Date) =>
    a.getFullYear() === b.getFullYear() &&
    a.getMonth() === b.getMonth() &&
    a.getDate() === b.getDate();
  if (sameDay(d, now)) return "Hoje";
  const yest = new Date(now);
  yest.setDate(now.getDate() - 1);
  if (sameDay(d, yest)) return "Ontem";
  return d.toLocaleDateString("pt-BR", { day: "numeric", month: "long" });
}

export function DateSeparator({ timestamp }: { timestamp: number }) {
  return (
    <div className="flex items-center justify-center my-2">
      <span className="text-[10px] uppercase tracking-wide text-harbor-ink/50 bg-harbor-surface/70 rounded-full px-3 py-1">
        {dayLabel(timestamp)}
      </span>
    </div>
  );
}

export default DateSeparator;
