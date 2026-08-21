/** Media-room authorization stays limited to an already paired device pair. */
import { afterEach, describe, expect, it } from "vitest";
import { SELF, env, reset } from "cloudflare:test";
import { pairKey } from "../src/util";

afterEach(async () => {
  await reset();
});

const BASE = "http://harbor.test";
const newId = () => crypto.randomUUID();

async function post(path: string, body: unknown, headers?: Record<string, string>) {
  const response = await SELF.fetch(BASE + path, {
    method: "POST",
    headers: { "Content-Type": "application/json", ...headers },
    body: JSON.stringify(body),
  });
  return { status: response.status, body: (await response.json()) as Record<string, unknown> };
}

async function register(device_id: string) {
  const result = await post("/register", { device_id });
  return result.body as { pairing_code: string; device_secret: string };
}

describe("media authorization", () => {
  it("proves a paired peer without exposing device secrets", async () => {
    const aid = newId();
    const bid = newId();
    const a = await register(aid);
    const b = await register(bid);
    const paired = await post("/pair", {
      device_id: bid,
      device_secret: b.device_secret,
      partner_code: a.pairing_code,
    });
    expect(paired.status).toBe(200);

    const registry = env.HARBOR_REGISTRY.get(env.HARBOR_REGISTRY.idFromName("harbor-registry"));
    await expect(registry.authorizeMedia(aid, a.device_secret, bid)).resolves.toEqual({
      authorized: true,
      role: "peer",
      partner_id: bid,
      pair_key: pairKey(aid, bid),
    });
    let error: unknown = null;
    try {
      await registry.authorizeMedia(aid, a.device_secret, newId());
    } catch (caught) {
      error = caught;
    }
    expect(error).toMatchObject({ status: 403 });
  });

  it("does not expose the authorization route until its secret is configured", async () => {
    const result = await post("/media-authorize", {});
    expect(result.status).toBe(503);
    expect(result.body.detail).toBe("media_auth_not_configured");
  });
});
