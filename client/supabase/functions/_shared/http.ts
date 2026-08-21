export const JSON_HEADERS = { "Content-Type": "application/json" };

function corsHeaders(request: Request): Record<string, string> {
  const configured = Deno.env.get("TURN_ALLOWED_ORIGINS")?.split(",").map((value) => value.trim()).filter(Boolean) ?? [];
  const origin = request.headers.get("origin");
  const allowed = configured.length === 0 ? "*" : origin && configured.includes(origin) ? origin : "null";
  return {
    "Access-Control-Allow-Origin": allowed,
    "Access-Control-Allow-Headers": "authorization, apikey, content-type",
    "Access-Control-Allow-Methods": "POST, OPTIONS",
    Vary: "Origin",
  };
}

export function json(request: Request, body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { ...JSON_HEADERS, ...corsHeaders(request) },
  });
}

export function options(request: Request): Response {
  return new Response(null, { status: 204, headers: corsHeaders(request) });
}

export function bearerToken(request: Request): string | null {
  const value = request.headers.get("authorization") ?? "";
  const match = /^Bearer\s+([^\s]+)$/i.exec(value);
  return match?.[1] ?? null;
}

export async function jsonBody(request: Request): Promise<Record<string, unknown> | null> {
  try {
    const value: unknown = await request.json();
    if (!value || typeof value !== "object" || Array.isArray(value)) return null;
    return value as Record<string, unknown>;
  } catch {
    return null;
  }
}
