import { describe, expect, it } from "vitest";
import { signMediaJwt, verifyMediaJwt } from "../../supabase/functions/_shared/jwt";

const claims = {
  sub: "device-a",
  role: "authenticated" as const,
  aud: "authenticated" as const,
  iss: "harbor-media" as const,
  media_topic: "harbor:voice:pair:device-a:device-b",
  media_peer_id: "device-b",
  iat: 1_700_000_000,
  exp: 1_700_000_300,
};

describe("media JWT contract", () => {
  it("signs a short-lived token that verifies with the same secret", async () => {
    const token = await signMediaJwt(claims, "test-secret");
    await expect(verifyMediaJwt(token, "test-secret", 1_700_000_100)).resolves.toEqual(claims);
    await expect(verifyMediaJwt(token, "wrong-secret", 1_700_000_100)).resolves.toBeNull();
  });

  it("rejects expired, overlong, and modified tokens", async () => {
    const expired = await signMediaJwt({ ...claims, exp: claims.iat + 10 }, "test-secret");
    await expect(verifyMediaJwt(expired, "test-secret", claims.iat + 11)).resolves.toBeNull();

    const overlong = await signMediaJwt({ ...claims, exp: claims.iat + 901 }, "test-secret");
    await expect(verifyMediaJwt(overlong, "test-secret", claims.iat + 10)).resolves.toBeNull();

    const token = await signMediaJwt(claims, "test-secret");
    const parts = token.split(".");
    parts[1] = `${parts[1]}x`;
    await expect(verifyMediaJwt(parts.join("."), "test-secret", 1_700_000_100)).resolves.toBeNull();
  });
});
