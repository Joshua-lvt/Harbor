/**
 * HarborRegistry — the singleton Durable Object that owns the GLOBAL concerns a
 * per-pair Durable Object cannot: an arbitrary `pairing_code → device` lookup and
 * `device → pair` routing. It is a faithful port of `server/app/pairing.py` +
 * `server/app/db.py` + `server/app/security.py`.
 *
 * What lives here (SQLite, one `devices` table):
 *   device identity, the per-device `secret` (channel auth credential), the outstanding
 *   `pairing_code`, the `partner_id` link, the deterministic `pair_key` (so both devices
 *   hash to one HarborPair), display name, public key, avatar.
 *
 * What does NOT live here (moved to HarborPair as live data):
 *   presence / last_seen — those are runtime-only and owned by the per-pair DO.
 *
 * Security: `secret` is the only credential and it lives ONLY here. The Worker
 * authenticates every WebSocket upgrade via `verifyDevice` BEFORE forwarding the upgraded
 * socket to HarborPair, which then trusts the connection — so HarborPair holds no secrets
 * and no E2E private keys ever touch the server. Private keys never leave the client.
 */
import { DurableObject } from "cloudflare:workers";

import { PairError } from "./protocol";
import { generateCode, newSecret, nowTs, pairKey, verifySecret } from "./util";

/** A row of the `devices` table (the columns this DO reads/writes). */
interface DeviceRow {
  id: string;
  secret: string;
  pairing_code: string | null;
  partner_id: string | null;
  pair_key: string | null;
  display_name: string | null;
  public_key: string | null;
  avatar: string | null;
  created_at: number;
}

const SCHEMA_V1 = `
CREATE TABLE IF NOT EXISTS devices (
    id            TEXT PRIMARY KEY,
    secret        TEXT NOT NULL,
    pairing_code  TEXT UNIQUE,
    partner_id    TEXT,
    pair_key      TEXT,
    display_name  TEXT,
    public_key    TEXT,
    avatar        TEXT,
    created_at    REAL NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_devices_code ON devices(pairing_code);
CREATE TABLE IF NOT EXISTS _schema_migrations (
    version     INTEGER PRIMARY KEY,
    applied_at  REAL NOT NULL
);
INSERT OR IGNORE INTO _schema_migrations (version, applied_at) VALUES (1, ?);
`;

const SELECT_DEVICE =
  "SELECT id, secret, pairing_code, partner_id, pair_key, display_name, public_key, avatar, created_at FROM devices WHERE id = ?";

export class HarborRegistry extends DurableObject<Env> {
  /** Memoized one-shot schema ensure per instantiation. CREATE IF NOT EXISTS is
   *  idempotent; after hibernation the promise is rebuilt and re-runs (fast, safe). */
  #schema: Promise<void> | null = null;

  private async ensureSchema(): Promise<void> {
    if (!this.#schema) {
      this.#schema = this.ctx.blockConcurrencyWhile(async () => {
        this.ctx.storage.sql.exec(SCHEMA_V1, nowTs());
      });
    }
    return this.#schema;
  }

  private q<T = Record<string, unknown>>(sql: string, ...params: unknown[]): T[] {
    return this.ctx.storage.sql.exec(sql, ...params).toArray() as T[];
  }

  private async getDevice(deviceId: string): Promise<DeviceRow | null> {
    await this.ensureSchema();
    const rows = this.q<DeviceRow>(SELECT_DEVICE, deviceId);
    return rows.length ? rows[0] : null;
  }

  private findIdByCode(code: string): string | null {
    const rows = this.q<{ id: string }>("SELECT id FROM devices WHERE pairing_code = ?", code);
    return rows.length ? rows[0].id : null;
  }

  /** Generate a pairing code not already outstanding (mirrors `_fresh_code`). */
  private freshCode(): string {
    for (let i = 0; i < 5; i++) {
      const code = generateCode();
      if (this.findIdByCode(code) === null) return code;
    }
    throw new PairError("could not allocate a unique code", 500);
  }

  /* ───────────────────────── HTTP-backed RPC ──────────────────────── */

  /** POST /register — create or refresh a device row; return a fresh code + secret.
   *  Re-registering an existing device re-issues secret + code but preserves the
   *  partner link (`pairing.py:53`). Avatar/pubkey optional so pre-E2E/pre-avatar
   *  clients keep working. */
  async register(
    deviceId: string,
    publicKey: string | null = null,
    avatar: string | null = null,
  ): Promise<{ pairing_code: string; device_secret: string }> {
    await this.ensureSchema();
    if (!deviceId || deviceId.length < 8) throw new PairError("bad device_id", 400);

    let code = "";
    const secret = newSecret();
    const now = nowTs();
    // Retry on pairing_code UNIQUE collision (the ON CONFLICT below handles id reuse;
    // a generated code colliding with another device's outstanding code still raises).
    for (let i = 0; i < 5; i++) {
      code = generateCode();
      try {
        this.ctx.storage.sql.exec(
          "INSERT INTO devices(id, secret, pairing_code, public_key, avatar, created_at) " +
            "VALUES (?, ?, ?, ?, ?, ?) " +
            "ON CONFLICT(id) DO UPDATE SET " +
            "  secret = excluded.secret, pairing_code = excluded.pairing_code, " +
            "  public_key = excluded.public_key, avatar = excluded.avatar",
          deviceId,
          secret,
          code,
          publicKey,
          avatar,
          now,
        );
        return { pairing_code: code, device_secret: secret };
      } catch (e) {
        // pairing_code collision → retry; anything else surfaces.
        if (!isUniqueViolation(e)) throw e;
      }
    }
    throw new PairError("could not allocate a unique code", 500);
  }

  /** POST /pair — authenticate the caller, look up the partner by code, retire both
   *  codes (single-use), cross-link partners, pin `pair_key` on both rows, bootstrap the
   *  HarborPair. Returns the partner's static info (`pairing.py:87`). */
  async pair(
    callerId: string,
    callerSecret: string,
    partnerCode: string,
  ): Promise<{
    partner_device_id: string;
    partner_name: string | null;
    partner_public_key: string | null;
    partner_avatar: string | null;
  }> {
    await this.ensureSchema();
    if (!partnerCode) throw new PairError("missing partner code", 400);

    const caller = await this.getDevice(callerId);
    if (caller === null || !verifySecret(caller.secret, callerSecret)) {
      throw new PairError("unauthorized", 401);
    }
    const partnerId = this.findIdByCode(partnerCode);
    if (partnerId === null) throw new PairError("invalid or expired code", 404);
    if (partnerId === callerId) throw new PairError("cannot pair with self", 400);

    const pk = pairKey(callerId, partnerId);

    // Retire both codes (single-use) and cross-link partners + pin pair_key, atomically
    // via the DO's single-threaded + write-coalesced storage.
    this.ctx.storage.sql.exec(
      "UPDATE devices SET pairing_code = NULL, pair_key = ? WHERE id IN (?, ?)",
      pk,
      callerId,
      partnerId,
    );
    this.ctx.storage.sql.exec(
      "UPDATE devices SET partner_id = ?, pair_key = ? WHERE id = ?",
      partnerId,
      pk,
      callerId,
    );
    this.ctx.storage.sql.exec(
      "UPDATE devices SET partner_id = ?, pair_key = ? WHERE id = ?",
      callerId,
      pk,
      partnerId,
    );

    // Bootstrap the per-pair DO (idempotent — writes both members rows). Awaiting here
    // guarantees the pair exists before the client connects its WebSocket post-/pair.
    const pairStub = this.env.HARBOR_PAIR.get(this.env.HARBOR_PAIR.idFromName(pk));
    await pairStub.bootstrap(callerId, partnerId);

    const partner = await this.getDevice(partnerId);
    return {
      partner_device_id: partnerId,
      partner_name: partner ? partner.display_name : null,
      partner_public_key: partner ? partner.public_key : null,
      partner_avatar: partner ? partner.avatar : null,
    };
  }

  /** POST /profile — update display_name and/or public_key and/or avatar.
   *  Avatar semantics: `null`/omitted → don't touch the stored value; `""` → clear it
   *  to NULL (absence); any data URL → set it. The FastAPI relay (`pairing.py:130`)
   *  stored the `""` sentinel verbatim; here we normalize `""` → NULL so the stored
   *  avatar is invariantly either a real data URL or NULL. This is wire-compatible:
   *  the client renders both `""` and `null` as the shark-mascot fallback
   *  (`Avatar.tsx: showImg = !!src`), and the clear signal on the wire is unchanged
   *  (`avatar: ""`). public_key / display_name follow the trivial "set when present" rule. */
  async setProfile(
    callerId: string,
    callerSecret: string,
    displayName: string | null = null,
    publicKey: string | null = null,
    avatar: string | null = null,
  ): Promise<{ ok: true }> {
    await this.ensureSchema();
    const me = await this.getDevice(callerId);
    if (me === null || !verifySecret(me.secret, callerSecret)) throw new PairError("unauthorized", 401);

    // `""` selects the avatar-bearing branch (clear) but stores NULL, not the sentinel.
    const avatarStored = avatar === "" ? null : avatar;

    // Four branches mirror `pairing.py:130-155` (which field set is present).
    if (publicKey !== null && avatar !== null) {
      this.ctx.storage.sql.exec(
        "UPDATE devices SET display_name = ?, public_key = ?, avatar = ? WHERE id = ?",
        displayName,
        publicKey,
        avatarStored,
        callerId,
      );
    } else if (publicKey !== null) {
      this.ctx.storage.sql.exec(
        "UPDATE devices SET display_name = ?, public_key = ? WHERE id = ?",
        displayName,
        publicKey,
        callerId,
      );
    } else if (avatar !== null) {
      this.ctx.storage.sql.exec(
        "UPDATE devices SET display_name = ?, avatar = ? WHERE id = ?",
        displayName,
        avatarStored,
        callerId,
      );
    } else {
      this.ctx.storage.sql.exec(
        "UPDATE devices SET display_name = ? WHERE id = ?",
        displayName,
        callerId,
      );
    }
    return { ok: true };
  }

  /** GET /partner — the passive partner's cold-start lookup. Returns the partner's
   *  STATIC info plus live `presence`/`last_seen` (fetched from HarborPair). Mirrors
   *  `pairing.py:159`, which read everything from the devices table in one shot. */
  async getPartnerInfo(
    callerId: string,
    callerSecret: string,
  ): Promise<{
    partner_device_id: string;
    partner_name: string | null;
    presence: string;
    last_seen: number | null;
    partner_public_key: string | null;
    partner_avatar: string | null;
  }> {
    await this.ensureSchema();
    const me = await this.getDevice(callerId);
    if (me === null || !verifySecret(me.secret, callerSecret)) throw new PairError("unauthorized", 401);
    if (!me.partner_id) throw new PairError("not paired", 404);
    const partner = await this.getDevice(me.partner_id);
    if (partner === null) throw new PairError("partner not found", 404);

    // Live presence/last_seen live in the per-pair DO; ask it.
    let presence = "offline";
    let lastSeen: number | null = null;
    if (me.pair_key) {
      try {
        const stub = this.env.HARBOR_PAIR.get(this.env.HARBOR_PAIR.idFromName(me.pair_key));
        const live = await stub.getPresence(me.partner_id);
        presence = live.presence;
        lastSeen = live.last_seen;
      } catch {
        // Pair DO unavailable → offline (cold start, never connected).
      }
    }
    return {
      partner_device_id: partner.id,
      partner_name: partner.display_name,
      presence,
      last_seen: lastSeen,
      partner_public_key: partner.public_key,
      partner_avatar: partner.avatar,
    };
  }

  /** GET /me — the caller's own state, for cold-start recovery after the partner
   *  unpaired us while we were offline (`pairing.py:179`). */
  async getMe(
    callerId: string,
    callerSecret: string,
  ): Promise<{ pairing_code: string | null; partner_id: string | null; display_name: string | null }> {
    await this.ensureSchema();
    const me = await this.getDevice(callerId);
    if (me === null || !verifySecret(me.secret, callerSecret)) throw new PairError("unauthorized", 401);
    return {
      pairing_code: me.pairing_code,
      partner_id: me.partner_id,
      display_name: me.display_name,
    };
  }

  /** POST /unpair — break the pairing bilaterally: clear the link + reissue fresh
   *  single-use codes to both, then tell HarborPair to drop its outbox and notify the
   *  live ex-partner over WS (`pairing.py:196` + `routes.py:60`). Returns the caller's
   *  new code; the ex-partner's new code is delivered to them over their WebSocket. */
  async unpair(
    callerId: string,
    callerSecret: string,
  ): Promise<{ ok: true; pairing_code: string; _partnerId: string | null }> {
    await this.ensureSchema();
    const me = await this.getDevice(callerId);
    if (me === null || !verifySecret(me.secret, callerSecret)) throw new PairError("unauthorized", 401);
    const partnerId = me.partner_id;
    if (!partnerId) throw new PairError("not paired", 404);

    const pk = me.pair_key!;
    const codeMe = this.freshCode();
    const codeThem = this.freshCode();

    this.ctx.storage.sql.exec(
      "UPDATE devices SET partner_id = NULL, pair_key = NULL, pairing_code = ? WHERE id = ?",
      codeMe,
      callerId,
    );
    this.ctx.storage.sql.exec(
      "UPDATE devices SET partner_id = NULL, pair_key = NULL, pairing_code = ? WHERE id = ?",
      codeThem,
      partnerId,
    );

    // Outbox + live notify are HarborPair's concern. Fire-and-await so the HTTP response
    // isn't blocked on the ex-partner's socket health.
    const stub = this.env.HARBOR_PAIR.get(this.env.HARBOR_PAIR.idFromName(pk));
    await stub.clearOutboxForPair();
    await stub.notifyUnpaired(partnerId, codeThem);

    return { ok: true, pairing_code: codeMe, _partnerId: partnerId };
  }

  /* ───────────────────────── WS-auth RPC ──────────────────────────── */

  /** Authenticate a device for the WebSocket upgrade. The Worker calls this BEFORE
   *  forwarding the upgraded socket to HarborPair, so HarborPair can trust the
   *  connection without ever holding the secret. Returns the `pair_key` so the Worker
   *  knows which HarborPair instance to hand the socket to. */
  async verifyDevice(
    deviceId: string,
    secret: string,
  ): Promise<{ ok: boolean; pair_key: string | null; partner_id: string | null }> {
    await this.ensureSchema();
    const me = await this.getDevice(deviceId);
    if (me === null || !verifySecret(me.secret, secret)) {
      return { ok: false, pair_key: null, partner_id: null };
    }
    return { ok: true, pair_key: me.pair_key, partner_id: me.partner_id };
  }
}

/** SQLite UNIQUE-violation detection by error message text (the DO SQLite surface raises
 *  on constraint violation but doesn't tag the error with a stable code we can switch on). */
function isUniqueViolation(e: unknown): boolean {
  const m = e instanceof Error ? e.message : String(e);
  return /unique/i.test(m);
}
