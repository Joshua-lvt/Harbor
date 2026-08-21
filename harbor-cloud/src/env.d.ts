/** Deployment secret supplied with `wrangler secret put`; absent in local dev until configured. */
interface Env {
  HARBOR_MEDIA_AUTH_TOKEN?: string;
}
