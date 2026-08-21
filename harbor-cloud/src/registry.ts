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
  mobile_code: string | null;
  mobile_code_expires: number | null;
  /** Registry-side target makes a lost /connect_mobile response retryable. */
  observer_target_id: string | null;
  partner_id: string | null;
  pair_key: string | null;
  display_name: string | null;
  public_key: string | null;
  avatar: string | null;
  created_at: number;
}

/** The pre-observer schema. Keep this baseline stable: CREATE IF NOT EXISTS does not
 * alter a table already persisted by an older Worker deployment. Additive columns are
 * applied by migrateSchema below instead. */
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

/** TTL for a mobile-linking code (~10 min; mirrors typical pairing ephemerality). */
const MOBILE_CODE_TTL_SECONDS = 10 * 60;

/** Mobile codes are short & readable (4 letters + 4 letters) but distinct from the
 *  HARBOR-XXXX-XXXX pairing format so one can't be mistaken for the other. */
const MOBILE_ALPHABET = "ABCDEFGHJKMNPQRSTUVWXYZ23456789";

function freshMobileCodeRandom(chars: number): string {
  // Rejection-sample until the requested length is complete. A bounded sample can
  // (rarely) run out of acceptable bytes and return a short code.
  let out = "";
  const max = Math.floor(0x100 / MOBILE_ALPHABET.length) * MOBILE_ALPHABET.length;
  while (out.length < chars) {
    const buf = new Uint8Array(Math.max(chars - out.length, 1) * 2);
    crypto.getRandomValues(buf);
    for (const v of buf) {
      if (v < max) {
        out += MOBILE_ALPHABET[v % MOBILE_ALPHABET.length];
        if (out.length === chars) break;
      }
    }
  }
  return out;
}

/** Shared column list so both `getDevice` (by id) and `findByMobileCode` (by code)
 *  return the same full row without duplicating the projection. */
const DEVICE_COLS =
  "id, secret, pairing_code, mobile_code, mobile_code_expires, observer_target_id, partner_id, pair_key, display_name, public_key, avatar, created_at";
const MOBILE_CODE_RE = /^[A-Z2-9]{4}-[A-Z2-9]{4}$/;

const SELECT_DEVICE = `SELECT ${DEVICE_COLS} FROM devices WHERE id = ?`;

export class HarborRegistry extends DurableObject<Env> {
  /** Memoized schema ensure per instantiation. The migration is synchronous inside
   *  blockConcurrencyWhile so no request can observe a half-added column. */
  #schema: Promise<void> | null = null;

  private async ensureSchema(): Promise<void> {
    if (!this.#schema) {
      this.#schema = this.ctx.blockConcurrencyWhile(async () => {
        this.ctx.storage.sql.exec(SCHEMA_V1, nowTs());
        const columns = new Set(
          this.ctx.storage.sql
            .exec("PRAGMA table_info(devices)")
            .toArray()
            .map((row) => String((row as { name: unknown }).name)),
        );
        if (!columns.has("mobile_code"))
          this.ctx.storage.sql.exec("ALTER TABLE devices ADD COLUMN mobile_code TEXT");
        if (!columns.has("mobile_code_expires"))
          this.ctx.storage.sql.exec("ALTER TABLE devices ADD COLUMN mobile_code_expires REAL");
        if (!columns.has("observer_target_id"))
          this.ctx.storage.sql.exec("ALTER TABLE devices ADD COLUMN observer_target_id TEXT");
        this.ctx.storage.sql.exec("CREATE INDEX IF NOT EXISTS idx_devices_mobile_code ON devices(mobile_code)");
        this.ctx.storage.sql.exec(
          "CREATE UNIQUE INDEX IF NOT EXISTS idx_devices_mobile_code_unique ON devices(mobile_code)",
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

  /** Generate a mobile code `XXXX-XXXX` not already outstanding (nor expired). */
  private freshMobileCode(): string {
    for (let i = 0; i < 5; i++) {
      const code = `${freshMobileCodeRandom(4)}-${freshMobileCodeRandom(4)}`;
      if (this.findByMobileCode(code) === null) return code;
    }
    throw new PairError("could not allocate a unique code", 500);
  }

  /** Resolve an unexpired mobile code to its owning device, else null. */
  private findByMobileCode(code: string): DeviceRow | null {
    const rows = this.q<DeviceRow>(`SELECT ${DEVICE_COLS} FROM devices WHERE mobile_code = ?`, code);
    const row = rows.length ? rows[0] : null;
    if (!row || !row.mobile_code_expires || row.mobile_code_expires <= nowTs()) return null;
    return row;
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
            "  mobile_code = NULL, mobile_code_expires = NULL, " +
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

  /** Move target-specific observers from a solo Pair DO to a newly-created pair DO.
   *  The Registry row is updated only after the destination membership is ready, and
   *  each operation is idempotent so a retry can finish a partially completed move. */
  private async rehomeObservers(oldPairKey: string | null, targetId: string, newPairKey: string): Promise<void> {
    if (!oldPairKey || oldPairKey === newPairKey) return;
    const oldStub = this.env.HARBOR_PAIR.get(this.env.HARBOR_PAIR.idFromName(oldPairKey));
    const newStub = this.env.HARBOR_PAIR.get(this.env.HARBOR_PAIR.idFromName(newPairKey));
    const observerIds = await oldStub.listObservers(targetId);
    for (const observerId of observerIds) {
      await newStub.addObserver(observerId, targetId);
      this.ctx.storage.sql.exec(
        "UPDATE devices SET pair_key = ? WHERE id = ? AND observer_target_id = ?",
        newPairKey,
        observerId,
        targetId,
      );
      await oldStub.removeObserver(observerId);
    }
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
    const partner = await this.getDevice(partnerId);
    if (!partner) throw new PairError("partner not found", 404);
    if (caller.observer_target_id || partner.observer_target_id)
      throw new PairError("observers cannot become peers", 409);
    if (caller.partner_id || partner.partner_id)
      throw new PairError("device already paired", 409);

    const pk = pairKey(callerId, partnerId);
    const oldCallerPairKey = caller.pair_key;
    const oldPartnerPairKey = partner.pair_key;

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
    await this.rehomeObservers(oldCallerPairKey, callerId, pk);
    await this.rehomeObservers(oldPartnerPairKey, partnerId, pk);

    return {
      partner_device_id: partnerId,
      partner_name: partner.display_name,
      partner_public_key: partner.public_key,
      partner_avatar: partner.avatar,
    };
  }

  /** POST /media-authorize — authenticate one paired peer for a private media room.
   * This endpoint is intended only for the Supabase Edge Function; it returns no
   * device secret and refuses observers, unpaired devices, and mismatched partners. */
  async authorizeMedia(
    callerId: string,
    callerSecret: string,
    partnerId: string,
  ): Promise<{ authorized: true; role: "peer"; partner_id: string; pair_key: string }> {
    await this.ensureSchema();
    const caller = await this.getDevice(callerId);
    if (caller === null || !verifySecret(caller.secret, callerSecret)) {
      throw new PairError("unauthorized", 401);
    }
    if (!partnerId || caller.partner_id !== partnerId || !caller.pair_key) {
      throw new PairError("media peer is not paired with caller", 403);
    }
    const partner = await this.getDevice(partnerId);
    if (!partner || partner.partner_id !== callerId || partner.pair_key !== caller.pair_key) {
      throw new PairError("media peer is not paired with caller", 403);
    }
    return { authorized: true, role: "peer", partner_id: partnerId, pair_key: caller.pair_key };
  }

  /** POST /mobile_code — authenticate the caller (the PC), mint a short-lived,
   *  single-use mobile-linking code, and store it on the caller's row. The code is
   *  DEDICATED (never usable in /pair) so leaking it can only grant presence/activity
   *  observation, never pairing takeover. Idempotent: calling again rotates the code.
   *  Returns the code + its expiry (epoch seconds). */
  async mintMobileCode(
    callerId: string,
    callerSecret: string,
  ): Promise<{ mobile_code: string; expires: number }> {
    await this.ensureSchema();
    const me = await this.getDevice(callerId);
    if (me === null || !verifySecret(me.secret, callerSecret)) throw new PairError("unauthorized", 401);

    const code = this.freshMobileCode();
    const expires = nowTs() + MOBILE_CODE_TTL_SECONDS;
    this.ctx.storage.sql.exec(
      "UPDATE devices SET mobile_code = ?, mobile_code_expires = ? WHERE id = ?",
      code,
      expires,
      callerId,
    );
    return { mobile_code: code, expires };
  }

  /** POST /connect_mobile — bind a mobile device as a receive-only OBSERVER of a PC.
   *  The caller (mobile) authenticates with its own secret, presents the PC's
   *  single-use `mobile_code`, resolves it to the target PC, reuses the target's
   *  `pair_key` (or derives a solo one if the PC is unpaired), links the observer to
   *  that HarborPair, and returns the target `device_id` the mobile should watch.
   *  The mobile_code is consumed (single-use); the mobile keeps its own pairing_code
   *  untouched (so it can still /pair normally if it chooses). */
  async bindObserver(
    callerId: string,
    callerSecret: string,
    mobileCode: string,
  ): Promise<{ target_id: string }> {
    await this.ensureSchema();
    if (!mobileCode || !MOBILE_CODE_RE.test(mobileCode))
      throw new PairError("invalid mobile_code", 400);
    const me = await this.getDevice(callerId);
    if (me === null || !verifySecret(me.secret, callerSecret)) throw new PairError("unauthorized", 401);
    if (me.partner_id) throw new PairError("paired devices cannot be observers", 409);

    // A lost HTTP response must be retryable without consuming a second code. The
    // binding is owned by the mobile row; re-assert the pair membership before returning.
    if (me.observer_target_id) {
      const existingTarget = await this.getDevice(me.observer_target_id);
      if (!existingTarget || !me.pair_key) throw new PairError("observer target not found", 409);
      const existingStub = this.env.HARBOR_PAIR.get(this.env.HARBOR_PAIR.idFromName(me.pair_key));
      await existingStub.addObserver(callerId, existingTarget.id);
      this.ctx.storage.sql.exec(
        "UPDATE devices SET mobile_code = NULL, mobile_code_expires = NULL " +
          "WHERE id = ? AND mobile_code = ? AND mobile_code_expires > ?",
        existingTarget.id,
        mobileCode,
        nowTs(),
      );
      return { target_id: existingTarget.id };
    }

    // Resolve the code to the target PC, rejecting expired/consumed codes.
    const target = this.findByMobileCode(mobileCode);
    if (target === null) throw new PairError("invalid or expired code", 404);
    if (target.id === callerId) throw new PairError("cannot observe self", 400);
    if (target.observer_target_id) throw new PairError("target is not a PC", 409);

    // Reuse the target's existing pair_key (a paired PC) or derive a solo key.
    const pk = target.pair_key ?? pairKey(target.id, target.id);
    const isSolo = !target.pair_key;
    if (isSolo) {
      // The target itself must authenticate into the solo Pair DO so a later
      // PC↔PC pairing can discover and rehome its observers.
      this.ctx.storage.sql.exec(
        "UPDATE devices SET pair_key = ? WHERE id = ?",
        pk,
        target.id,
      );
    }
    const stub = this.env.HARBOR_PAIR.get(this.env.HARBOR_PAIR.idFromName(pk));
    if (isSolo) await stub.bootstrap(target.id, target.id);

    // Persist the intended binding before the cross-DO call. If that call fails, a
    // retry sees observer_target_id and re-asserts membership instead of orphaning it.
    this.ctx.storage.sql.exec(
      "UPDATE devices SET pair_key = ?, observer_target_id = ? WHERE id = ?",
      pk,
      target.id,
      callerId,
    );
    await stub.addObserver(callerId, target.id);

    // The Registry DO is single-threaded. Conditional consumption prevents a future
    // implementation from consuming a code that was rotated between lookup and bind.
    this.ctx.storage.sql.exec(
      "UPDATE devices SET mobile_code = NULL, mobile_code_expires = NULL " +
        "WHERE id = ? AND mobile_code = ? AND mobile_code_expires > ?",
      target.id,
      mobileCode,
      nowTs(),
    );
    if (this.findByMobileCode(mobileCode)) throw new PairError("mobile_code was already consumed", 409);
    return { target_id: target.id };
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
    const soloMe = pairKey(callerId, callerId);
    const soloThem = pairKey(partnerId, partnerId);
    const oldStub = this.env.HARBOR_PAIR.get(this.env.HARBOR_PAIR.idFromName(pk));
    const observersMe = await oldStub.listObservers(callerId);
    const observersThem = await oldStub.listObservers(partnerId);
    const codeMe = this.freshCode();
    const codeThem = this.freshCode();

    // Keep observer links attached to their PC after the bilateral peer link is
    // removed. The new solo pair DOs are bootstrapped below before observers move.
    this.ctx.storage.sql.exec(
      "UPDATE devices SET partner_id = NULL, pair_key = ?, pairing_code = ? WHERE id = ?",
      soloMe,
      codeMe,
      callerId,
    );
    this.ctx.storage.sql.exec(
      "UPDATE devices SET partner_id = NULL, pair_key = ?, pairing_code = ? WHERE id = ?",
      soloThem,
      codeThem,
      partnerId,
    );

    // Outbox + live notify are HarborPair's concern.
    await oldStub.clearOutboxForPair();
    await oldStub.notifyUnpaired(partnerId, codeThem);

    const move = async (targetId: string, soloKey: string, observerIds: string[]) => {
      const soloStub = this.env.HARBOR_PAIR.get(this.env.HARBOR_PAIR.idFromName(soloKey));
      await soloStub.bootstrap(targetId, targetId);
      for (const observerId of observerIds) {
        await soloStub.addObserver(observerId, targetId);
        this.ctx.storage.sql.exec(
          "UPDATE devices SET pair_key = ? WHERE id = ? AND observer_target_id = ?",
          soloKey,
          observerId,
          targetId,
        );
        await oldStub.removeObserver(observerId);
      }
    };
    await move(callerId, soloMe, observersMe);
    await move(partnerId, soloThem, observersThem);

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
