/**
 * HarborRegistry + Worker-router tests (HTTP path).
 *
 * Hits the real Worker router via `SELF.fetch`, which delegates to the singleton
 * HarborRegistry DO — so this exercises request validation, auth
 * (constant-time secret), the PairError→HTTP-status mapping, code single-use
 * semantics, profile set/clear, /me, /partner, and /unpair end-to-end through
 * the bundle that ships to prod.
 *
 * Storage isolation is per test *file* in the pool (each file gets a fresh
 * Miniflare). Within this file we wipe DO storage between cases with `reset()`
 * so the singleton Registry's `devices` table doesn't bleed across tests.
 */
import { afterEach, describe, expect, it } from "vitest";
import { SELF, reset } from "cloudflare:test";

afterEach(async () => {
  await reset();
});

const BASE = "http://harbor.test";

async function post(path: string, body: unknown) {
  const res = await SELF.fetch(BASE + path, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });
  return { status: res.status, body: (await res.json()) as Record<string, unknown> };
}

async function get(path: string) {
  const res = await SELF.fetch(BASE + path);
  return { status: res.status, body: (await res.json()) as Record<string, unknown> };
}

/** Fresh unique device id for a single test (UUIDs here are >= 8 chars). */
const newId = () =>
  typeof crypto !== "undefined" && "randomUUID" in crypto
    ? crypto.randomUUID()
    : `dev-${Math.random().toString(36).slice(2)}-${Math.random().toString(36).slice(2)}`;

const CODE_RE = /^HARBOR-[A-Z2-9]{4}-[A-Z2-9]{4}$/;

/** Register a fresh device and return its id + code + secret together (the HTTP
 *  /register response omits device_id, so tests carry the id from the request). */
async function register(opts?: { public_key?: string; avatar?: string }) {
  const device_id = newId();
  const r = await post("/register", {
    device_id,
    ...(opts?.public_key ? { public_key: opts.public_key } : {}),
    ...(opts?.avatar ? { avatar: opts.avatar } : {}),
  });
  return {
    device_id,
    status: r.status,
    pairing_code: r.body.pairing_code as string,
    device_secret: r.body.device_secret as string,
  };
}

describe("GET /health", () => {
  it("returns {status:'ok'}", async () => {
    const { status, body } = await get("/health");
    expect(status).toBe(200);
    expect(body.status).toBe("ok");
  });
});

describe("POST /register", () => {
  it("issues a HARBOR-XXXX-XXXX code + a device_secret", async () => {
    const { status, body } = await post("/register", { device_id: newId() });
    expect(status).toBe(200);
    expect(CODE_RE.test(body.pairing_code as string)).toBe(true);
    expect(typeof body.device_secret).toBe("string");
    expect((body.device_secret as string).length).toBeGreaterThan(20);
  });

  it("accepts optional public_key + avatar", async () => {
    const { status, body } = await post("/register", {
      device_id: newId(),
      public_key: "pk-base64",
      avatar: "data:image/jpeg;base64,AAAA",
    });
    expect(status).toBe(200);
    expect(CODE_RE.test(body.pairing_code as string)).toBe(true);
  });

  it("400s on a too-short device_id", async () => {
    const { status, body } = await post("/register", { device_id: "short" });
    expect(status).toBe(400);
    expect(body.detail).toBe("invalid_body");
  });

  it("re-registering an existing device re-issues code + secret", async () => {
    const id = newId();
    const r1 = await post("/register", { device_id: id });
    const r2 = await post("/register", { device_id: id });
    expect(r2.status).toBe(200);
    expect(r2.body.pairing_code).not.toBe(r1.body.pairing_code);
    expect(r2.body.device_secret).not.toBe(r1.body.device_secret);
  });
});

describe("POST /pair", () => {
  it("links two devices: B pairs with A's code → partner info returned", async () => {
    const aid = newId();
    const bid = newId();
    const a = await post("/register", { device_id: aid });
    const b = await post("/register", { device_id: bid });
    const r = await post("/pair", {
      device_id: bid,
      device_secret: b.body.device_secret as string,
      partner_code: a.body.pairing_code as string,
    });
    expect(r.status).toBe(200);
    expect(r.body.partner_device_id).toBe(aid);
    expect(r.body.partner_name).toBeNull();
    expect(r.body.partner_public_key).toBeNull();
    expect(r.body.partner_avatar).toBeNull();
  });

  it("rejects an unknown pairing code with 404 (invalid or expired)", async () => {
    const bid = newId();
    const b = await post("/register", { device_id: bid });
    const r = await post("/pair", {
      device_id: bid,
      device_secret: b.body.device_secret as string,
      partner_code: "HARBOR-NOPE-CODE",
    });
    expect(r.status).toBe(404);
  });

  it("retires both codes (single-use): a third device presenting A's old code → 404", async () => {
    const aid = newId();
    const bid = newId();
    const cid = newId();
    const a = await post("/register", { device_id: aid });
    const b = await post("/register", { device_id: bid });
    const c = await post("/register", { device_id: cid });
    const codeA = a.body.pairing_code as string;
    await post("/pair", {
      device_id: bid,
      device_secret: b.body.device_secret as string,
      partner_code: codeA,
    });
    // A's code is now retired; C trying it → 404 ("invalid or expired code").
    const r = await post("/pair", {
      device_id: cid,
      device_secret: c.body.device_secret as string,
      partner_code: codeA,
    });
    expect(r.status).toBe(404);
  });

  it("rejects self-pair with 400", async () => {
    const aid = newId();
    const a = await post("/register", { device_id: aid });
    const r = await post("/pair", {
      device_id: aid,
      device_secret: a.body.device_secret as string,
      partner_code: a.body.pairing_code as string,
    });
    expect(r.status).toBe(400);
  });

  it("rejects a bad device_secret with 401", async () => {
    const aid = newId();
    const bid = newId();
    const a = await post("/register", { device_id: aid });
    await post("/register", { device_id: bid });
    const r = await post("/pair", {
      device_id: bid,
      device_secret: "wrong-secret",
      partner_code: a.body.pairing_code as string,
    });
    expect(r.status).toBe(401);
  });
});

describe("POST /profile", () => {
  it("sets display_name + public_key + avatar, then /partner surfaces them", async () => {
    const aid = newId();
    const bid = newId();
    const a = await post("/register", { device_id: aid });
    const b = await post("/register", { device_id: bid });
    // A publishes name + pubkey + avatar.
    const pr = await post("/profile", {
      device_id: aid,
      device_secret: a.body.device_secret as string,
      display_name: "Josué",
      public_key: "pk-a-base64",
      avatar: "data:image/jpeg;base64,AA",
    });
    expect(pr.status).toBe(200);
    expect(pr.body.ok).toBe(true);

    await post("/pair", {
      device_id: bid,
      device_secret: b.body.device_secret as string,
      partner_code: a.body.pairing_code as string,
    });
    const partner = await get(
      `/partner?device_id=${bid}&secret=${b.body.device_secret}`,
    );
    expect(partner.status).toBe(200);
    expect(partner.body.partner_device_id).toBe(aid);
    expect(partner.body.partner_name).toBe("Josué");
    expect(partner.body.partner_public_key).toBe("pk-a-base64");
    expect(partner.body.partner_avatar).toBe("data:image/jpeg;base64,AA");
    // No WS yet → presence offline.
    expect(partner.body.presence).toBe("offline");
  });

  it("clears avatar with an empty string (distinct from null=skip)", async () => {
    const aid = newId();
    const a = await post("/register", {
      device_id: aid,
      avatar: "data:image/jpeg;base64,SET",
    });
    // Clear the avatar.
    await post("/profile", {
      device_id: aid,
      device_secret: a.body.device_secret as string,
      display_name: "X",
      avatar: "",
    });
    // /me shows display_name; /partner isn't paired so read /me-style via re-register
    // isn't ideal. Instead re-register would re-issue. Use a paired partner to read.
    const bid = newId();
    const b = await post("/register", { device_id: bid });
    await post("/pair", {
      device_id: bid,
      device_secret: b.body.device_secret as string,
      partner_code: a.body.pairing_code as string,
    });
    const partner = await get(`/partner?device_id=${bid}&secret=${b.body.device_secret}`);
    expect(partner.body.partner_avatar).toBeNull();
  });

  it("401s on a bad secret", async () => {
    const r = await post("/profile", {
      device_id: newId(),
      device_secret: "nope",
      display_name: "X",
    });
    expect(r.status).toBe(401);
  });
});

describe("GET /me", () => {
  it("returns the caller's code + partner link + display_name", async () => {
    const aid = newId();
    const bid = newId();
    const a = await post("/register", { device_id: aid });
    const b = await post("/register", { device_id: bid });
    await post("/profile", {
      device_id: aid,
      device_secret: a.body.device_secret as string,
      display_name: "A",
    });
    await post("/pair", {
      device_id: bid,
      device_secret: b.body.device_secret as string,
      partner_code: a.body.pairing_code as string,
    });
    const me = await get(`/me?device_id=${aid}&secret=${a.body.device_secret}`);
    expect(me.status).toBe(200);
    expect(me.body.pairing_code).toBeNull(); // retired after pairing
    expect(me.body.partner_id).toBe(bid);
    expect(me.body.display_name).toBe("A");
  });

  it("401s on a bad secret", async () => {
    const r = await get(`/me?device_id=${newId()}&secret=nope`);
    expect(r.status).toBe(401);
  });
});

describe("GET /partner", () => {
  it("404s when the caller isn't paired (no partner link yet)", async () => {
    const a = await register();
    const r = await get(`/partner?device_id=${a.device_id}&secret=${a.device_secret}`);
    expect(r.status).toBe(404);
  });

  it("404s (not paired) for a real, paired-then-unpaired device", async () => {
    const aid = newId();
    const bid = newId();
    const a = await post("/register", { device_id: aid });
    const b = await post("/register", { device_id: bid });
    await post("/pair", {
      device_id: bid,
      device_secret: b.body.device_secret as string,
      partner_code: a.body.pairing_code as string,
    });
    // Unpair via A, then B's /partner should be not-paired.
    await post("/unpair", {
      device_id: aid,
      device_secret: a.body.device_secret as string,
    });
    const r = await get(`/partner?device_id=${bid}&secret=${b.body.device_secret}`);
    expect(r.status).toBe(404);
  });
});

describe("POST /unpair", () => {
  it("breaks the link bilaterally + reissues a code to the caller", async () => {
    const aid = newId();
    const bid = newId();
    const a = await post("/register", { device_id: aid });
    const b = await post("/register", { device_id: bid });
    await post("/pair", {
      device_id: bid,
      device_secret: b.body.device_secret as string,
      partner_code: a.body.pairing_code as string,
    });
    const r = await post("/unpair", {
      device_id: aid,
      device_secret: a.body.device_secret as string,
    });
    expect(r.status).toBe(200);
    expect(r.body.ok).toBe(true);
    expect(CODE_RE.test(r.body.pairing_code as string)).toBe(true);
    // Both devices now unpaired — A's /me shows no partner.
    const me = await get(`/me?device_id=${aid}&secret=${a.body.device_secret}`);
    expect(me.body.partner_id).toBeNull();
    expect(me.body.pairing_code).toBe(r.body.pairing_code);
    // B is also unpaired.
    const meB = await get(`/me?device_id=${bid}&secret=${b.body.device_secret}`);
    expect(meB.body.partner_id).toBeNull();
    expect(meB.body.pairing_code).not.toBeNull(); // B got a fresh code via notifyUnpaired
  });

  it("404s when the caller isn't paired", async () => {
    const a = await register();
    const r = await post("/unpair", {
      device_id: a.device_id,
      device_secret: a.device_secret,
    });
    expect(r.status).toBe(404);
  });

  it("401s on a bad secret", async () => {
    const r = await post("/unpair", { device_id: newId(), device_secret: "nope" });
    expect(r.status).toBe(401);
  });
});
