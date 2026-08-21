/**
 * Persistence-compatibility tests for observer schema migrations.
 *
 * The normal test fixtures create a fresh DO, which would not catch a deploy over
 * an existing V1 SQLite table. These cases deliberately pre-create the old tables,
 * invoke the real DO code, and verify both additive columns and preserved rows.
 */
import { afterEach, describe, expect, it } from "vitest";
import { env } from "cloudflare:workers";
import { SELF, reset, runInDurableObject } from "cloudflare:test";
import { pairKey } from "../src/util";

const BASE = "http://harbor.test";
const registryStub = () => env.HARBOR_REGISTRY.get(env.HARBOR_REGISTRY.idFromName("harbor-registry"));

async function post(path: string, body: unknown) {
  const res = await SELF.fetch(BASE + path, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });
  return { status: res.status, body: (await res.json()) as Record<string, unknown> };
}

afterEach(async () => {
  await reset();
});

describe("Durable Object V1 → observer migrations", () => {
  it("adds Registry columns and preserves a legacy device", async () => {
    const legacyId = "legacy-registry-device";
    await runInDurableObject(registryStub(), (_instance, state) => {
      state.storage.sql.exec(`
        CREATE TABLE devices (
          id TEXT PRIMARY KEY,
          secret TEXT NOT NULL,
          pairing_code TEXT UNIQUE,
          partner_id TEXT,
          pair_key TEXT,
          display_name TEXT,
          public_key TEXT,
          avatar TEXT,
          created_at REAL NOT NULL
        );
        CREATE TABLE _schema_migrations (version INTEGER PRIMARY KEY, applied_at REAL NOT NULL);
        INSERT INTO _schema_migrations(version, applied_at) VALUES (1, 1);
        INSERT INTO devices(id, secret, pairing_code, display_name, created_at)
          VALUES (?, ?, ?, ?, ?);
      `, legacyId, "legacy-secret", "HARBOR-AAAA-BBBB", "Legacy", 1);
    });

    const fresh = await post("/register", { device_id: "fresh-registry-device" });
    expect(fresh.status).toBe(200);

    const inspected = await runInDurableObject(registryStub(), (_instance, state) => {
      const columns = state.storage.sql.exec("PRAGMA table_info(devices)").toArray()
        .map((r) => String((r as { name: unknown }).name));
      const row = state.storage.sql.exec(
        "SELECT id, secret, pairing_code, display_name, mobile_code, observer_target_id FROM devices WHERE id = ?",
        legacyId,
      ).toArray()[0] as Record<string, unknown>;
      const versions = state.storage.sql.exec("SELECT version FROM _schema_migrations ORDER BY version")
        .toArray().map((r) => Number((r as { version: unknown }).version));
      return { columns, row, versions };
    });

    expect(inspected.columns).toEqual(expect.arrayContaining(["mobile_code", "mobile_code_expires", "observer_target_id"]));
    expect(inspected.row).toMatchObject({
      id: legacyId,
      secret: "legacy-secret",
      pairing_code: "HARBOR-AAAA-BBBB",
      display_name: "Legacy",
      mobile_code: null,
      observer_target_id: null,
    });
    expect(inspected.versions).toContain(1);
    expect(inspected.versions).toContain(2);

    // Re-entering the DO must not attempt a duplicate ALTER or change the row.
    await post("/register", { device_id: "another-registry-device" });
  });

  it("adds Pair role/target columns and backfills legacy members as peers", async () => {
    const aid = "legacy-pair-device-a";
    const bid = "legacy-pair-device-b";
    const pk = pairKey(aid, bid);
    const stub = env.HARBOR_PAIR.get(env.HARBOR_PAIR.idFromName(pk));
    await runInDurableObject(stub, (_instance, state) => {
      state.storage.sql.exec(`
        CREATE TABLE members (
          device_id TEXT PRIMARY KEY,
          pair_key TEXT NOT NULL,
          last_presence TEXT NOT NULL DEFAULT 'offline',
          last_seen REAL,
          pending_grace_until REAL
        );
        CREATE TABLE outbox (
          seq INTEGER PRIMARY KEY AUTOINCREMENT,
          from_id TEXT NOT NULL,
          to_id TEXT NOT NULL,
          payload TEXT NOT NULL,
          ts REAL NOT NULL
        );
        CREATE TABLE _schema_migrations (version INTEGER PRIMARY KEY, applied_at REAL NOT NULL);
        INSERT INTO _schema_migrations(version, applied_at) VALUES (1, 1);
        INSERT INTO members(device_id, pair_key) VALUES (?, ?), (?, ?);
      `, aid, pk, bid, pk);
    });

    expect(await stub.getPresence(aid)).toEqual({ presence: "offline", last_seen: null });
    const inspected = await runInDurableObject(stub, (_instance, state) => {
      const columns = state.storage.sql.exec("PRAGMA table_info(members)").toArray()
        .map((r) => String((r as { name: unknown }).name));
      const rows = state.storage.sql.exec(
        "SELECT device_id, role, observed_device_id FROM members ORDER BY device_id",
      ).toArray() as Array<Record<string, unknown>>;
      const versions = state.storage.sql.exec("SELECT version FROM _schema_migrations ORDER BY version")
        .toArray().map((r) => Number((r as { version: unknown }).version));
      return { columns, rows, versions };
    });

    expect(inspected.columns).toEqual(expect.arrayContaining(["role", "observed_device_id"]));
    expect(inspected.rows).toEqual([
      { device_id: aid, role: "peer", observed_device_id: null },
      { device_id: bid, role: "peer", observed_device_id: null },
    ]);
    expect(inspected.versions).toContain(1);
    expect(inspected.versions).toContain(2);
    await stub.getPresence(bid);
  });
});
