# Media Edge Functions

These functions are deployed separately from the Harbor client bundle. They do not
accept or return a permanent TURN provider key.

Required Supabase secrets:

- `HARBOR_MEDIA_AUTH_URL` — private/additive Harbor endpoint (for example `https://harbor.example/media-authorize`).
- `HARBOR_MEDIA_AUTH_TOKEN` — server-to-server token configured as a Cloudflare Worker secret with the same value.
- `MEDIA_JWT_SECRET` — signing secret accepted by Supabase Realtime private-channel authorization.
- `TURN_ICE_SERVERS_JSON` — JSON array containing only the static TURN `urls`, `username`, and `credential` values.
- Optional `MEDIA_TOKEN_TTL_SECONDS`, `TURN_TTL_SECONDS`, and `TURN_ALLOWED_ORIGINS`.

Set secrets with the Supabase CLI or dashboard; never place their values in this
repository or in a `VITE_*` variable:

```sh
supabase secrets set HARBOR_MEDIA_AUTH_URL=... HARBOR_MEDIA_AUTH_TOKEN=... \
  MEDIA_JWT_SECRET=... TURN_ICE_SERVERS_JSON='[...]'
supabase functions deploy media-session
supabase functions deploy turn-credentials
```

`media-session` accepts a Harbor `device_id`, `device_secret`, pair-derived
`room_id`, and partner id over HTTPS. It asks the private Harbor authorization
endpoint to verify that the caller is a paired peer (observers/mobile identities
must be rejected), then returns only a short-lived Realtime JWT. The device
secret never appears in the response.

`turn-credentials` requires that JWT and checks `sub`, `media_peer_id`, and
`media_topic` before returning short-lived TURN credentials. Configure a private
Realtime channel and apply the policies in `../schema.sql`; do not make the
voice channel public as a workaround for a JWT or RLS failure.
