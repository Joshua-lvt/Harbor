/**
 * notificationStore — the in-app, Harbor-styled notification history (Bug 4).
 *
 * The user wants Harbor's OWN notification surface, not the OS default toasts.
 * This singleton records every received notification (chat message, presence
 * change, quick-message) and persists it to SQLite (table `notifications`,
 * migrated in Rust — see lib.rs migration v5). The NotificationsScreen renders
 * this list in a custom Harbor layout. The OS toast fires independently (see
 * services/notify.ts) whenever the relevant toggle is on — this store is the
 * persistent in-app history, not a replacement for the OS alert.
 *
 * Pattern mirrors localDb.ts: a thin async wrapper over the SQL plugin with an
 * in-memory cache so subscribers get instant updates. `add` is fire-and-forget
 * (never blocks the receive path); `list`/`markRead`/`clear` are the read API.
 */
import { getDb } from "../lib/localDb";

export type NotificationKind = "message" | "presence" | "quick";

export interface StoredNotification {
  id: string;
  kind: NotificationKind;
  title: string;
  body: string;
  icon?: string;
  timestamp: number;
  read: boolean;
}

type ChangeHandler = (list: StoredNotification[]) => void;

const MAX_KEPT = 200;

class NotificationStore {
  private cache: StoredNotification[] = [];
  private loaded = false;
  private handlers = new Set<ChangeHandler>();

  private async ensureLoaded(): Promise<void> {
    if (this.loaded) return;
    const db = await getDb();
    const rows = await db.select<
      Array<{
        id: string;
        kind: string;
        title: string;
        body: string;
        icon: string | null;
        timestamp: number;
        read: number;
      }>
    >(
      "SELECT id, kind, title, body, icon, timestamp, read FROM notifications ORDER BY timestamp DESC LIMIT $1",
      [MAX_KEPT],
    );
    this.cache = rows.map((r) => ({
      id: r.id,
      kind: r.kind as NotificationKind,
      title: r.title,
      body: r.body,
      icon: r.icon ?? undefined,
      timestamp: r.timestamp,
      read: r.read === 1,
    }));
    this.loaded = true;
  }

  /** Record a received notification. Fire-and-forget: never blocks the caller
   *  (the receive path must stay fast). Dedupes by id (INSERT OR IGNORE). */
  async add(n: Omit<StoredNotification, "read">): Promise<void> {
    await this.ensureLoaded();
    const item: StoredNotification = { ...n, read: false };
    this.cache = [item, ...this.cache.filter((x) => x.id !== item.id)].slice(0, MAX_KEPT);
    try {
      const db = await getDb();
      await db.execute(
        "INSERT OR IGNORE INTO notifications(id, kind, title, body, icon, timestamp, read) VALUES ($1,$2,$3,$4,$5,$6,0)",
        [item.id, item.kind, item.title, item.body, item.icon ?? null, item.timestamp],
      );
    } catch {
      /* DB unavailable — the in-memory cache still serves this session. */
    }
    this.emit();
  }

  async list(): Promise<StoredNotification[]> {
    await this.ensureLoaded();
    return this.cache;
  }

  async markRead(id: string): Promise<void> {
    await this.ensureLoaded();
    this.cache = this.cache.map((x) => (x.id === id ? { ...x, read: true } : x));
    try {
      const db = await getDb();
      await db.execute("UPDATE notifications SET read = 1 WHERE id = $1", [id]);
    } catch {
      /* non-fatal */
    }
    this.emit();
  }

  async markAllRead(): Promise<void> {
    await this.ensureLoaded();
    this.cache = this.cache.map((x) => ({ ...x, read: true }));
    try {
      const db = await getDb();
      await db.execute("UPDATE notifications SET read = 1");
    } catch {
      /* non-fatal */
    }
    this.emit();
  }

  async clear(): Promise<void> {
    this.cache = [];
    try {
      const db = await getDb();
      await db.execute("DELETE FROM notifications");
    } catch {
      /* non-fatal */
    }
    this.emit();
  }

  /** Subscribe to the list; runs the handler once immediately with the current
   *  list (after the async load resolves). Returns unsubscribe. */
  onChange(h: ChangeHandler): () => void {
    this.handlers.add(h);
    void this.list().then(h);
    return () => this.handlers.delete(h);
  }

  private emit(): void {
    for (const h of this.handlers) h(this.cache);
  }
}

export const notificationStore = new NotificationStore();
