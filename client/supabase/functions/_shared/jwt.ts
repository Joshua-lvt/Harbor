export interface MediaJwtClaims {
  sub: string;
  role: "authenticated";
  aud: "authenticated";
  iss: "harbor-media";
  media_topic: string;
  media_peer_id: string;
  iat: number;
  exp: number;
}

type JsonRecord = Record<string, unknown>;

function encode(value: Uint8Array): string {
  let binary = "";
  for (const byte of value) binary += String.fromCharCode(byte);
  return btoa(binary).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/g, "");
}

function decode(value: string): Uint8Array {
  const normalized = value.replace(/-/g, "+").replace(/_/g, "/");
  const padded = normalized + "=".repeat((4 - (normalized.length % 4)) % 4);
  const binary = atob(padded);
  return Uint8Array.from(binary, (char) => char.charCodeAt(0));
}

function parsePart(value: string): JsonRecord | null {
  try {
    const parsed: unknown = JSON.parse(new TextDecoder().decode(decode(value)));
    return parsed && typeof parsed === "object" && !Array.isArray(parsed)
      ? (parsed as JsonRecord)
      : null;
  } catch {
    return null;
  }
}

export async function signMediaJwt(
  claims: MediaJwtClaims,
  secret: string,
): Promise<string> {
  const header = encode(new TextEncoder().encode(JSON.stringify({ alg: "HS256", typ: "JWT" })));
  const payload = encode(new TextEncoder().encode(JSON.stringify(claims)));
  const input = new TextEncoder().encode(`${header}.${payload}`);
  const key = await crypto.subtle.importKey(
    "raw",
    new TextEncoder().encode(secret),
    { name: "HMAC", hash: "SHA-256" },
    false,
    ["sign"],
  );
  const signature = new Uint8Array(await crypto.subtle.sign("HMAC", key, input));
  return `${header}.${payload}.${encode(signature)}`;
}

export async function verifyMediaJwt(
  token: string,
  secret: string,
  nowSeconds = Math.floor(Date.now() / 1000),
): Promise<MediaJwtClaims | null> {
  const parts = token.split(".");
  if (parts.length !== 3 || !secret) return null;
  const header = parsePart(parts[0]);
  const payload = parsePart(parts[1]);
  if (!header || !payload || header.alg !== "HS256" || header.typ !== "JWT") return null;

  try {
    const key = await crypto.subtle.importKey(
      "raw",
      new TextEncoder().encode(secret),
      { name: "HMAC", hash: "SHA-256" },
      false,
      ["verify"],
    );
    const valid = await crypto.subtle.verify(
      "HMAC",
      key,
      decode(parts[2]),
      new TextEncoder().encode(`${parts[0]}.${parts[1]}`),
    );
    if (!valid) return null;
  } catch {
    return null;
  }

  if (
    typeof payload.sub !== "string" ||
    payload.role !== "authenticated" ||
    payload.aud !== "authenticated" ||
    payload.iss !== "harbor-media" ||
    typeof payload.media_topic !== "string" ||
    typeof payload.media_peer_id !== "string" ||
    typeof payload.iat !== "number" ||
    typeof payload.exp !== "number" ||
    !Number.isSafeInteger(payload.iat) ||
    !Number.isSafeInteger(payload.exp) ||
    payload.exp <= nowSeconds ||
    payload.iat > nowSeconds + 60 ||
    payload.exp - payload.iat > 15 * 60
  ) {
    return null;
  }
  return payload as unknown as MediaJwtClaims;
}
