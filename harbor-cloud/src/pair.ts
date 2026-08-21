/**
 * HarborPair — one Durable Object per paired pair of devices (`idFromName(pair_key)`).
 *
 * This is the coordination atom the project requires: exactly two devices, nothing
 * public, no groups. It is a faithful port of `server/app/ws.py`'s ConnectionManager +
 * `_handle` + outbox/flush, rebuilt on the WebSocket **Hibernation API** so the DO
 * accrues no GB-s while the two devices sit idle (Harbor's common state — the widget is
 * "always on" but mostly quiet).
 *
 * State split (decided against the Hibernation constraints):
 *   SQLite (survives eviction): `members` (id, pair_key, last_presence, last_seen,
 *     pending_grace_until) and `outbox` (buffered offline chat). `getPresence` /
 *     `last_seen` reads come from here so an RPC wakes a hibernated pair and returns
 *     correct data.
 *   Hibernation attachment (`serializeAttachment`, ≤16KB, survives hibernation while
 *     the socket is healthy): just `{ device_id }` — enough to identify the sender in
 *     `webSocketMessage` after a hibernate-restore, without touching SQLite.
 *   Derived (never persisted): live connectedness — `ctx.getWebSockets(device_id)`.
 *
 * The only intentional behavior change from FastAPI: offline presence is pushed after a
 *  ~30s grace window (REGRA: don't mark offline on transient drops), whereas FastAPI
 *  pushed it immediately on disconnect. `/partner` polls still read `offline` promptly.
 *
 * This DO holds NO secrets (the Worker authenticates via HarborRegistry.verifyDevice
 * before forwarding the upgrade) and NO E2E private keys — it only forwards the opaque
 * base64 `enc` ciphertext verbatim. The server never decrypts. Audio stays P2P.
 */
import { DurableObject } from "cloudflare:workers";

import { ServerMessage, validateClientMessage } from "./protocol";
import { nowTs, parseFrame } from "./util";

interface MemberRow {
  device_id: string;
  pair_key: string;
  role: "peer" | "observer";
  /** Set only for observers; identifies the exact PC they monitor. */
  observed_device_id: string | null;
  last_presence: string;
  last_seen: number | null;
  pending_grace_until: number | null;
}

interface OutboxRow {
  seq: number;
  from_id: string;
  to_id: string;
  payload: string;
  ts: number;
}

/** Stable pre-observer schema. Existing DOs need ALTER TABLE migrations, not a
 * changed CREATE IF NOT EXISTS declaration. */
const SCHEMA_V1 = `
CREATE TABLE IF NOT EXISTS members (
    device_id            TEXT PRIMARY KEY,
    pair_key             TEXT NOT NULL,
    last_presence        TEXT NOT NULL DEFAULT 'offline',
    last_seen            REAL,
    pending_grace_until  REAL
);
CREATE TABLE IF NOT EXISTS outbox (
    seq       INTEGER PRIMARY KEY AUTOINCREMENT,
    from_id   TEXT NOT NULL,
    to_id     TEXT NOT NULL,
    payload   TEXT NOT NULL,
    ts        REAL NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_outbox_to ON outbox(to_id, ts);
CREATE TABLE IF NOT EXISTS _schema_migrations (
    version     INTEGER PRIMARY KEY,
    applied_at  REAL NOT NULL
);
INSERT OR IGNORE INTO _schema_migrations (version, applied_at) VALUES (1, ?);
`;

/** Degrees → milliseconds; the offline grace window (REGRA: don't flap on small drops). */
const GRACE_MS_DEFAULT = 30_000;
/** Outbox TTL in days (FastAPI: 7). Daily sweep cadence for the alarm reschedule. */
const TTL_SWEEP_MS = 24 * 60 * 60 * 1000;

export class HarborPair extends DurableObject<Env> {
  #schema: Promise<void> | null = null;

  private async ensureSchema(): Promise<void> {
    if (!this.#schema) {
      this.#schema = this.ctx.blockConcurrencyWhile(async () => {
        this.ctx.storage.sql.exec(SCHEMA_V1, nowTs());
        const columns = new Set(
          this.ctx.storage.sql
            .exec("PRAGMA table_info(members)")
            .toArray()
            .map((row) => String((row as { name: unknown }).name)),
        );
        if (!columns.has("role"))
          this.ctx.storage.sql.exec("ALTER TABLE members ADD COLUMN role TEXT NOT NULL DEFAULT 'peer'");
        if (!columns.has("observed_device_id"))
          this.ctx.storage.sql.exec("ALTER TABLE members ADD COLUMN observed_device_id TEXT");
        this.ctx.storage.sql.exec(
          "CREATE INDEX IF NOT EXISTS idx_members_observer_target " +
            "ON members(role, observed_device_id)",
        );
        this.ctx.storage.sql.exec(
          "INSERT OR IGNORE INTO _schema_migrations (version, applied_at) VALUES (2, ?)",
          nowTs(),
        );
      });
    }
    return this.#schema;
  }

  private q<T = Record<string, unknown>>(sql: string, ...params: unknown[]): T[] {
    return this.ctx.storage.sql.exec(sql, ...params).toArray() as T[];
  }

  private graceMs(): number {
    const v = this.env.HARBOR_OFFLINE_GRACE_MS;
    const n = v ? Number(v) : NaN;
    return Number.isFinite(n) && n > 0 ? n : GRACE_MS_DEFAULT;
  }

  private ttlDays(): number {
    const v = this.env.HARBOR_OFFLINE_MSG_TTL_DAYS;
    const n = v ? Number(v) : NaN;
    return Number.isFinite(n) && n > 0 ? n : 7;
  }

  /** The other PEER member of this pair (exactly one, when present). Observers
   *  (`role='observer'`) are excluded from peer routing so they never displace a
   *  real partner: a mobile observer added to a solo PC must not become its
   *  "partner" nor steal its would-be peer's messages. */
  private partnerOf(deviceId: string): string | null {
    const rows = this.q<{ device_id: string }>(
      "SELECT device_id FROM members WHERE device_id != ? AND role = 'peer'",
      deviceId,
    );
    return rows.length ? rows[0].device_id : null;
  }

  /** Observers explicitly bound to this source PC. A target-specific query is
   *  important: an observer of A must not receive B's activity in an A:B pair. */
  private observersOf(sourceId: string): string[] {
    return this.q<{ device_id: string }>(
      "SELECT device_id FROM members WHERE role = 'observer' AND observed_device_id = ?",
      sourceId,
    ).map((r) => r.device_id);
  }

  private getMember(deviceId: string): MemberRow | null {
    const rows = this.q<MemberRow>("SELECT * FROM members WHERE device_id = ?", deviceId);
    return rows.length ? rows[0] : null;
  }

  /** Live connectedness derived from the Hibernation registry — never persisted. */
  private isOnline(deviceId: string): boolean {
    return this.ctx.getWebSockets(deviceId).length > 0;
  }

  /** Send a ServerMessage to a specific device's live (hibernating-OK) socket(s).
   *  Returns true iff at least one send succeeded — the chat-ack contract (`ws.py:38`). */
  private sendTo(deviceId: string, msg: ServerMessage): boolean {
    const sockets = this.ctx.getWebSockets(deviceId);
    let delivered = false;
    for (const ws of sockets) {
      try {
        ws.send(JSON.stringify(msg));
        delivered = true;
      } catch {
        // Socket is gone — the Hibernation runtime will invoke webSocketClose.
      }
    }
    return delivered;
  }

  /** Fan a ServerMessage out to every observer in the pair except `sourceId`
   *  (the device whose event triggered the push — normally the peer that sent it).
   *  Skips observers that are offline; a solo PC with no observer is a no-op. */
  private sendToObservers(sourceId: string, msg: ServerMessage): void {
    for (const o of this.observersOf(sourceId)) this.sendTo(o, msg);
  }

  /* ─────────────────────────── RPC ──────────────────────────────── */

  /** Idempotent: write both members rows so the pair survives hibernation/bootstrap.
   *  Called by HarborRegistry.pair once the link is pinned. */
  async bootstrap(a: string, b: string): Promise<void> {
    await this.ensureSchema();
    const pk = this.ctx.id.name;
    // Existing rows are preserved (INSERT OR IGNORE) so a re-bootstrap never
    // downgrades a member's role (e.g. an observer stays an observer).
    this.ctx.storage.sql.exec(
      "INSERT OR IGNORE INTO members(device_id, pair_key, role, last_presence) VALUES (?, ?, 'peer', 'offline')",
      a,
      pk,
    );
    this.ctx.storage.sql.exec(
      "INSERT OR IGNORE INTO members(device_id, pair_key, role, last_presence) VALUES (?, ?, 'peer', 'offline')",
      b,
      pk,
    );
  }

  /** Add or repair a receive-only observer membership. The target is persisted so
   *  fan-out and cold-start snapshots are scoped to the PC that minted the code. */
  async addObserver(mobileId: string, targetId: string): Promise<void> {
    await this.ensureSchema();
    const pk = this.ctx.id.name;
    const target = this.getMember(targetId);
    if (!target || target.role !== "peer") throw new Error("observer target is not a peer");
    const existing = this.getMember(mobileId);
    if (existing?.role === "peer") throw new Error("device is already a peer");
    this.ctx.storage.sql.exec(
      "INSERT INTO members(device_id, pair_key, role, observed_device_id, last_presence) " +
        "VALUES (?, ?, 'observer', ?, 'offline') " +
        "ON CONFLICT(device_id) DO UPDATE SET pair_key = excluded.pair_key, " +
        "role = 'observer', observed_device_id = excluded.observed_device_id",
      mobileId,
      pk,
      targetId,
    );
  }

  /** Return observers for a target before a Registry pair-key transition. */
  async listObservers(targetId: string): Promise<string[]> {
    await this.ensureSchema();
    return this.q<{ device_id: string }>(
      "SELECT device_id FROM members WHERE role = 'observer' AND observed_device_id = ?",
      targetId,
    ).map((r) => r.device_id);
  }

  /** Remove an observer from an obsolete pair and close its old sockets. */
  async removeObserver(mobileId: string): Promise<void> {
    await this.ensureSchema();
    for (const ws of this.ctx.getWebSockets(mobileId)) {
      try {
        ws.close(4001, "observer_rehomed");
      } catch {
        /* already closed */
      }
    }
    this.ctx.storage.sql.exec(
      "DELETE FROM members WHERE device_id = ? AND role = 'observer'",
      mobileId,
    );
  }

  /** RPC for GET /partner — live presence/last_seen for one member.
   *  Wakes a hibernated pair; reads from SQLite so the answer is correct at rest. */
  async getPresence(
    deviceId: string,
  ): Promise<{ presence: "online" | "away" | "offline"; last_seen: number | null }> {
    await this.ensureSchema();
    const m = this.getMember(deviceId);
    if (!m) return { presence: "offline", last_seen: null };
    // If connected, the persisted `last_presence` is the user's current self-state
    // (online/away); if disconnected, it's offline regardless of the stored value.
    const presence = this.isOnline(deviceId)
      ? ((m.last_presence === "away" ? "away" : "online") as "online" | "away")
      : "offline";
    return { presence, last_seen: m.last_seen };
  }

  /** RPC for POST /unpair — drop buffered messages between the now-ex-pair and push the
   *  `unpaired` envelope to the live ex-partner carrying THEIR new code (`routes.py:76`). */
  async notifyUnpaired(exPartnerId: string, newCode: string): Promise<void> {
    await this.ensureSchema();
    this.sendTo(exPartnerId, { type: "unpaired", pairing_code: newCode, ts: nowTs() });
  }

  /** RPC for POST /unpair — clear the outbox between these two devices (`pairing.py:234`).
 *  Only PEER rows participate in chat/outbox; observers never have buffered mail, so
 *  restrict the sweep to `role='peer'` to avoid picking up an observer as a "member". */
  async clearOutboxForPair(): Promise<void> {
    await this.ensureSchema();
    const ids = this.q<{ device_id: string }>(
      "SELECT device_id FROM members WHERE role = 'peer'",
    );
    if (ids.length < 2) return;
    const [a, b] = [ids[0].device_id, ids[1].device_id];
    this.ctx.storage.sql.exec(
      "DELETE FROM outbox WHERE (from_id = ? AND to_id = ?) OR (from_id = ? AND to_id = ?)",
      a,
      b,
      b,
      a,
    );
  }

  /* ───────────────────────── WS upgrade ─────────────────────────── */

  /** The Worker forwards the already-authenticated upgrade here. We accept the socket
   *  into Hibernation, tag it with device_id, run the connect logic (presence online,
   *  notify partner, flush this device's outbox), and return the 101 handshake. */
  async fetch(request: Request): Promise<Response> {
    await this.ensureSchema();
    const url = new URL(request.url);
    const deviceId = url.searchParams.get("device_id");
    if (!deviceId) return new Response("missing device_id", { status: 400 });

    const member = this.getMember(deviceId);
    // The Worker verified via Registry; a member row should exist post-bootstrap. If
    // not, the pair was never bootstrapped (or unpaired) — refuse.
    if (!member) return new Response("forbidden", { status: 403 });

    // Duplicate connection of the same device → close the old, accept the new (REGRA 6).
    for (const old of this.ctx.getWebSockets(deviceId)) {
      try {
        old.close(4409, "duplicate_connection");
      } catch {
        // ignore
      }
    }

    const pair = new WebSocketPair();
    const client = pair[0];
    const server = pair[1];
    this.ctx.acceptWebSocket(server, [deviceId]);
    server.serializeAttachment({ device_id: deviceId });

    const isObserver = member.role === "observer";
    if (isObserver) {
      // Observer connect: receive-only. No presence persistence (we'd overwrite our
      // own offline row with online), no partner notify, no outbox flush (the
      // outbox only holds peer→peer chat, never observer mail). Refresh the exact
      // target PC's CURRENT presence so an A observer never boots from B's state.
      const targetId = member.observed_device_id;
      const target = targetId ? this.getMember(targetId) : null;
      const targetLive = targetId ? this.isOnline(targetId) : false;
      if (targetId && target) {
        this.sendTo(deviceId, {
          type: "presence",
          device_id: targetId,
          state: targetLive ? (target.last_presence === "away" ? "away" : "online") : "offline",
          ts: nowTs(),
          ...(target.last_seen !== null ? { last_seen: target.last_seen } : {}),
        });
      }
      return new Response(null, { status: 101, webSocket: client });
    }

    // Connect logic — mirrors `ws.py:250-259`, with one fix: do NOT blindly
    // flip presence to "online" on every connect. A reconnecting device whose
    // user is genuinely away has `last_presence = "away"` (still within the
    // grace window); forcing "online" here makes the partner's status dot flap
    // online→away on every dropped-and-reopened socket — exactly the flicker the
    // 30s notify-debounce works around, but the UI dot (Home/Chat/Widget) has no
    // such debounce so it visibly oscillates. Preserve the user's last
    // self-reported state across reconnects; only a genuine offline→connected
    // transition (fresh boot, or reconnect AFTER grace already pushed offline)
    // announces "online". The client re-asserts its current presence on every
    // `onopen` regardless, so a drifted stored value is corrected within a RTT.
    const announce = (member.last_presence === "offline" ? "online" : member.last_presence) as
      | "online"
      | "away";
    this.ctx.storage.sql.exec(
      "UPDATE members SET last_presence = ?, last_seen = ?, pending_grace_until = NULL WHERE device_id = ?",
      announce,
      nowTs(),
      deviceId,
    );
    const partnerId = this.partnerOf(deviceId);
    if (partnerId) {
      this.sendTo(partnerId, {
        type: "presence",
        device_id: deviceId,
        state: announce,
        ts: nowTs(),
      });
    }
    // Fan the peer's connect out to observers too (runs even without a partner —
    // a solo PC still reports its liveness to a linked observer).
    this.sendToObservers(deviceId, {
      type: "presence",
      device_id: deviceId,
      state: announce,
      ts: nowTs(),
    });
    this.flushOutboxFor(deviceId);

    return new Response(null, { status: 101, webSocket: client });
  }

  /* ─────────────────────── Hibernation handlers ─────────────────── */

  /** Per-message dispatch — a verbatim port of `ws._handle` (`ws.py:117`), including
   *  the `enc` opaque-passthrough, ack semantics, heartbeat-touches-last_seen-only,
   *  unknown-state ignoring, and the swallow-on-error contract. */
  async webSocketMessage(ws: WebSocket, message: ArrayBuffer | string): Promise<void> {
    await this.ensureSchema();
    const attachment = ws.deserializeAttachment() as { device_id: string } | null;
    const deviceId = attachment?.device_id;
    if (!deviceId) return; // Should not happen; we tagged at accept.

    // Apply the same byte cap before parsing on every path, including observers.
    const len = typeof message === "string" ? message.length : message.byteLength;
    const cap = Number(this.env.HARBOR_MAX_FRAME_BYTES) || 262_144;
    if (len > cap) {
      this.sendTo(deviceId, { type: "error", reason: "frame_too_large" });
      return;
    }

    // Observers are receive-only: accept ONLY heartbeats (to keep `last_seen` fresh
    // so the relay/partner polling sees the observer as alive and to satisfy the
    // mobile client's own ping). Any other inbound message is ignored so a mobile
    // observer can never push presence/activity/chat back into the PC's pair.
    const mRow = this.getMember(deviceId);
    const isObserver = mRow?.role === "observer";
    if (isObserver) {
      const parsed = parseFrame(message);
      if (parsed === null) return;
      const v = validateClientMessage(parsed);
      if (!v.ok) return;
      if (v.msg.type !== "heartbeat") return;
      this.ctx.storage.sql.exec(
        "UPDATE members SET last_seen = ? WHERE device_id = ?",
        nowTs(),
        deviceId,
      );
      return;
    }

    const parsed = parseFrame(message);
    if (parsed === null) return; // bad JSON → silently skip (`ws.py:266`)
    const v = validateClientMessage(parsed);
    if (!v.ok) {
      this.sendTo(deviceId, v.msg);
      return;
    }
    const msg = v.msg;

    try {
      switch (msg.type) {
        case "heartbeat":
          this.ctx.storage.sql.exec(
            "UPDATE members SET last_seen = ? WHERE device_id = ?",
            nowTs(),
            deviceId,
          );
          return;

        case "presence": {
          // Only online/away survive validation; unknown states are ignored.
          this.ctx.storage.sql.exec(
            "UPDATE members SET last_presence = ?, last_seen = ? WHERE device_id = ?",
            msg.state,
            nowTs(),
            deviceId,
          );
          this.sendToObservers(deviceId, {
            type: "presence",
            device_id: deviceId,
            state: msg.state,
            ts: nowTs(),
          });
          const p = this.partnerOf(deviceId);
          if (p)
            this.sendTo(p, {
              type: "presence",
              device_id: deviceId,
              state: msg.state,
              ts: nowTs(),
            });
          return;
        }

        case "typing": {
          const p = this.partnerOf(deviceId);
          if (p)
            this.sendTo(p, {
              type: "typing",
              device_id: deviceId,
              // Validation defaults a missing state to "start" (protocol.ts); the
              // ServerMessage union is non-optional, so assert it here.
              state: msg.state ?? "start",
              ts: nowTs(),
            });
          return;
        }

        case "activity": {
          this.sendToObservers(deviceId, {
            type: "activity",
            device_id: deviceId,
            app: msg.app,
            ts: nowTs(),
          });
          const p = this.partnerOf(deviceId);
          if (p)
            this.sendTo(p, {
              type: "activity",
              device_id: deviceId,
              app: msg.app,
              ts: nowTs(),
            });
          return;
        }

        case "profile_update": {
          // Transient real-time push (the persistent source of truth stays the
          // HTTP POST /profile). Relay verbatim like `activity`; the relay is
          // key-blind to display_name+avatar and forwards them untouched.
          const p = this.partnerOf(deviceId);
          if (p)
            this.sendTo(p, {
              type: "profile_update",
              device_id: deviceId,
              display_name: msg.display_name,
              avatar: msg.avatar,
              ts: nowTs(),
            });
          return;
        }

        case "activity_icon": {
          // Transient one-per-exe icon push (the exe NAME still travels in the
          // existing `activity` event every time; the icon payload goes once).
          // Relay verbatim like `activity`.
          const p = this.partnerOf(deviceId);
          if (p)
            this.sendTo(p, {
              type: "activity_icon",
              device_id: deviceId,
              app: msg.app,
              icon: msg.icon,
              ts: nowTs(),
            });
          return;
        }

        case "voice_signal": {
          // Signaling only — never buffered, never persisted, server never touches media.
          const p = this.partnerOf(deviceId);
          if (p)
            this.sendTo(p, {
              type: "voice_signal",
              device_id: deviceId,
              kind: msg.kind,
              data: msg.data,
              ts: nowTs(),
            });
          return;
        }

        case "chat": {
          this.handleChat(deviceId, msg);
          return;
        }

        case "last_seen": {
          const p = this.partnerOf(deviceId);
          if (!p) return;
          const pm = this.getMember(p);
          this.sendTo(deviceId, {
            type: "last_seen",
            device_id: p,
            last_seen: pm ? pm.last_seen : null,
            presence: pm ? (this.isOnline(p) ? (pm.last_presence === "away" ? "away" : "online") : "offline") : "offline",
          });
          return;
        }

        default:
          // Exhaustive; unmatched is a type error at compile time.
          return;
      }
    } catch {
      // A single bad message must not kill the socket (`ws.py:271`).
    }
  }

  /** Chat forwarding + ack + offline buffering — the most consequential path. Port of
   *  `ws.py:175-218`. `enc` is forwarded verbatim with NO `text`/`image` injected; the
   *  plaintext path carries `text` + an optional `data:image/` URL. */
  private handleChat(
    deviceId: string,
    msg: { type: "chat"; id: string } & ({ enc: string } | { text: string; image?: string }),
  ): void {
    const ts = nowTs();
    const partnerId = this.partnerOf(deviceId);

    // Build the forwarded payload exactly as FastAPI did (`ws.py:188-210`).
    let payload: Record<string, unknown>;
    if ("enc" in msg) {
      payload = { type: "chat", id: msg.id, from: deviceId, enc: msg.enc, ts };
    } else {
      payload = { type: "chat", id: msg.id, from: deviceId, text: msg.text, ts };
      if (typeof msg.image === "string" && msg.image.startsWith("data:image/")) {
        payload.image = msg.image;
      }
    }

    if (!partnerId) {
      // No partner linked — ack not-delivered, don't buffer (`ws.py:216`).
      this.sendTo(deviceId, { type: "ack", id: msg.id, delivered: false });
      return;
    }

    const delivered = this.sendTo(partnerId, payload as unknown as ServerMessage);
    if (!delivered) {
      // Buffer the undelivered payload for the partner's reconnect.
      this.ctx.storage.sql.exec(
        "INSERT INTO outbox(from_id, to_id, payload, ts) VALUES (?, ?, ?, ?)",
        deviceId,
        partnerId,
        JSON.stringify(payload),
        ts,
      );
    }
    this.sendTo(deviceId, { type: "ack", id: msg.id, delivered });
  }

  /** Deliver buffered messages addressed to this device, then delete them + late-ack
   *  the ORIGINAL sender (`ws.py:77-104`). Called on connect (and could be re-run if
   *  needed). */
  private flushOutboxFor(deviceId: string): void {
    const rows = this.q<OutboxRow>(
      "SELECT seq, from_id, to_id, payload, ts FROM outbox WHERE to_id = ? ORDER BY ts ASC",
      deviceId,
    );
    for (const r of rows) {
      const payload = JSON.parse(r.payload) as ServerMessage;
      const ok = this.sendTo(deviceId, payload);
      if (ok) {
        this.ctx.storage.sql.exec("DELETE FROM outbox WHERE seq = ?", r.seq);
        // Late-ack the original sender so their bubble flips from "sending" to "delivered".
        if (typeof (payload as { id?: unknown }).id === "string") {
          this.sendTo(r.from_id, { type: "ack", id: (payload as { id: string }).id, delivered: true });
        }
      }
    }
  }

  /** On close: schedule a grace alarm. If the device reconnects before it fires, the
   *  alarm sees it online and skips the offline push (canceling the flap). */
  async webSocketClose(ws: WebSocket, code: number, reason: string, wasClean: boolean): Promise<void> {
    await this.ensureSchema();
    const attachment = ws.deserializeAttachment() as { device_id: string } | null;
    const deviceId = attachment?.device_id;
    if (!deviceId) return;

    // Observers are observe-only: no persisted presence to broadcast, so no grace
    // alarm and no offline fan-out. (Their own disconnect is invisible to peers.)
    const mRow = this.getMember(deviceId);
    if (mRow?.role === "observer") return;

    // Only schedule the grace if this was the device's last live socket (a duplicate
    // close from the replaced connection shouldn't flap a freshly-accepted one).
    if (!this.isOnline(deviceId)) {
      const fireAt = nowTs() + this.graceMs() / 1000;
      this.ctx.storage.sql.exec(
        "UPDATE members SET pending_grace_until = ?, last_seen = ? WHERE device_id = ?",
        fireAt,
        nowTs(),
        deviceId,
      );
      this.scheduleNextAlarm();
    }
  }

  async webSocketError(ws: WebSocket): Promise<void> {
    // Treat as a close; the runtime will also invoke webSocketClose.
    await this.webSocketClose(ws, 1011, "error", false);
  }

  /* ─────────────────────────── Alarms ───────────────────────────── */

  /** Idempotent: process grace expirations (push offline), sweep stale outbox, and
   *  reschedule to the earliest pending grace or the next daily TTL sweep. Alarms may
   *  fire more than once — every branch is safe to repeat. */
  async alarm(): Promise<void> {
    await this.ensureSchema();
    const now = nowTs();

    // 1) Grace expirations: members whose grace window elapsed and who are still gone.
    const graceRows = this.q<MemberRow>(
      "SELECT * FROM members WHERE pending_grace_until IS NOT NULL AND pending_grace_until <= ?",
      now,
    );
    for (const m of graceRows) {
      if (this.isOnline(m.device_id)) {
        // Reconnected mid-grace → cancel the flap, clear the pending flag.
        this.ctx.storage.sql.exec(
          "UPDATE members SET pending_grace_until = NULL WHERE device_id = ?",
          m.device_id,
        );
        continue;
      }
      this.ctx.storage.sql.exec(
        "UPDATE members SET last_presence = 'offline', pending_grace_until = NULL, last_seen = ? WHERE device_id = ?",
        now,
        m.device_id,
      );
      // Fan the offline out to the peer AND any observers.
      this.sendToObservers(m.device_id, {
        type: "presence",
        device_id: m.device_id,
        state: "offline",
        ts: now,
        last_seen: now,
      });
      const partner = this.partnerOf(m.device_id);
      if (partner) {
        this.sendTo(partner, {
          type: "presence",
          device_id: m.device_id,
          state: "offline",
          ts: now,
          last_seen: now,
        });
      }
    }

    // 2) TTL sweep: outbox rows older than the TTL (`db.sweep_outbox`).
    const cutoff = now - this.ttlDays() * 86400;
    this.ctx.storage.sql.exec("DELETE FROM outbox WHERE ts < ?", cutoff);

    // 3) Reschedule: earliest remaining grace, else a daily TTL fallback.
    this.scheduleNextAlarm();
  }

  /** Set the alarm to the earliest of: any pending grace_until, or now + TTL_SWEEP_MS. */
  private scheduleNextAlarm(): void {
    const rows = this.q<{ pending_grace_until: number }>(
      "SELECT pending_grace_until FROM members WHERE pending_grace_until IS NOT NULL ORDER BY pending_grace_until ASC LIMIT 1",
    );
    const now = nowTs();
    const nextGrace = rows.length ? rows[0].pending_grace_until : null;
    const nextTtl = now + TTL_SWEEP_MS / 1000;
    const next = nextGrace !== null ? Math.min(nextGrace, nextTtl) : nextTtl;
    if (next > now) this.ctx.storage.setAlarm(next * 1000);
  }
}
