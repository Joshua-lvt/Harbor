/**
 * Local message history + pending-outbox persistence via @tauri-apps/plugin-sql
 * (SQLite backend). The schema is migrated by the Rust side on startup; here we
 * just open the handle and provide thin query helpers.
 *
 * Migrations are defined in Rust (client/src-tauri/src/lib.rs) so the SQL
 * plugin applies them in a transaction at boot.
 */
import Database from "@tauri-apps/plugin-sql";

const DB_URI = "sqlite:harbor.db";

let db: Database | null = null;

export async function getDb(): Promise<Database> {
  if (!db) {
    db = await Database.load(DB_URI);
  }
  return db;
}

/**
 * Normalize an epoch timestamp to MILLISECONDS.
 *
 * The relay stamps every `chat` event with `ts` in SECONDS (Python `time.time()`
 * on the FastAPI relay, `nowTs() = Date.now() / 1000` on the Cloudflare Worker —
 * see harbor-cloud/src/util.ts). Outgoing messages, however, are stamped with
 * `Date.now()` already in milliseconds (useChat.send). So a raw incoming `ts` is
 * ~1.78e9 while an outgoing `created_at` is ~1.78e12 — three orders of magnitude
 * apart.
 *
 * Storing them side by side broke both the sort order (received messages, being
 * numerically smaller, always sorted to the top) and the date separator
 * (`new Date(seconds)` reads them as ms and renders ~21 January 1970). This
 * helper unifies everything to ms at the read/write boundaries.
 *
 * It is also a one-time legacy fix: rows written before this normalization land
 * in the DB as seconds, and `asMs` converts them on load. The threshold is
 * `1e12` ms = 2001-09-09 — Harbor has no messages from before 2001, so any value
 * below it is unambiguously seconds. Safe well past the year 20000.
 */
export function asMs(ts: number): number {
  return ts > 0 && ts < 1e12 ? ts * 1000 : ts;
}

/**
 * Insert a message I just sent (optimistic). `status` is "sending" until the
 * relay acks delivery, then updated to "delivered".
 */
export async function insertOutgoing(
  id: string,
  partnerId: string,
  text: string,
  createdAt: number,
  image?: string | null,
): Promise<void> {
  const conn = await getDb();
  await conn.execute(
    "INSERT INTO messages(id, partner_id, text, from_me, created_at, status, image) VALUES ($1,$2,$3,1,$4,'sending',$5)",
    [id, partnerId, text, createdAt, image ?? null],
  );
}

/** Insert a message received from the partner. */
export async function insertIncoming(
  id: string,
  partnerId: string,
  text: string,
  createdAt: number,
  image?: string | null,
): Promise<void> {
  const conn = await getDb();
  await conn.execute(
    "INSERT OR IGNORE INTO messages(id, partner_id, text, from_me, created_at, status, image) VALUES ($1,$2,$3,0,$4,'delivered',$5)",
    [id, partnerId, text, createdAt, image ?? null],
  );
}

/** Mark one of my outgoing messages as delivered (relay ack received). */
export async function markDelivered(id: string): Promise<void> {
  const conn = await getDb();
  await conn.execute("UPDATE messages SET status = 'delivered' WHERE id = $1", [id]);
}

/** Load the conversation with the partner, oldest first. */
export async function loadMessages(partnerId: string, limit = 200): Promise<StoredRow[]> {
  const conn = await getDb();
  return conn.select<StoredRow[]>(
    "SELECT id, partner_id, text, from_me, created_at, status, image FROM messages WHERE partner_id = $1 ORDER BY created_at ASC LIMIT $2",
    [partnerId, limit],
  );
}

/** Wipe ALL local message history — used when unpairing. The relay already
 *  drops its undelivered outbox between the two; this clears our local copy of
 *  the conversation so a re-pair with someone new starts from a clean slate. */
export async function clearAllMessages(): Promise<void> {
  const conn = await getDb();
  await conn.execute("DELETE FROM messages");
}

export interface StoredRow {
  id: string;
  partner_id: string;
  text: string;
  from_me: number;
  created_at: number;
  status: string;
  image: string | null;
}
