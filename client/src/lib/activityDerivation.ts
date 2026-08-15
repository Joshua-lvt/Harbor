/**
 * ActivityTracker — converts the partner's raw `activity` event stream into
 * history rows (Feature 3). One tracker per partner session (App.tsx owns it
 * for the app lifetime while paired + connected).
 *
 * The relay forwards `activity { app }` envelopes every ~4s whenever the
 * partner's foreground app changes (or `app: null` when their stream goes
 * idle / sharing is off). We derive a chronological history from that stream:
 *
 *  - `open`    — the partner switched TO `app` (from null / first app seen).
 *  - `switch`  — the partner switched from `prevExe` directly to `app`.
 *  - `close`   — the partner went idle (`app` null) while an app was open.
 *
 * Durations come from the gap between consecutive events (ts is the relay's
 * wall-clock seconds → we convert to ms here, matching useChat's server ts).
 * History starts from the first observed event after connect (no backfill).
 */
import { insertActivityEvent } from "./activityHistory";

export class ActivityTracker {
  private partnerId: string;
  private prevExe: string | null = null;
  private prevStartedAt: number | null = null;

  constructor(partnerId: string) {
    this.partnerId = partnerId;
  }

  /** Process one `activity` event. `app` is the lowercased exe (null = idle);
   *  `tsMs` is the event timestamp in epoch MILLISECONDS. */
  async onActivity(app: string | null, tsMs: number): Promise<void> {
    // No transition if the exe didn't change (keepalive echo).
    if (app === this.prevExe) return;

    if (this.prevExe != null) {
      // We were in an app; now leaving it.
      if (app == null) {
        // Idle close: record a `close` with the duration we held the app.
        await insertActivityEvent(
          this.partnerId,
          this.prevExe,
          "close",
          this.prevStartedAt!,
          tsMs,
        );
        this.prevExe = null;
        this.prevStartedAt = null;
      } else {
        // Direct switch from prevExe → app. Record the switch with the duration
        // we held prevExe, then open the new app.
        await insertActivityEvent(
          this.partnerId,
          this.prevExe,
          "switch",
          this.prevStartedAt!,
          tsMs,
        );
        await insertActivityEvent(this.partnerId, app, "open", tsMs);
        this.prevExe = app;
        this.prevStartedAt = tsMs;
      }
    } else {
      // We were idle; now an app is open.
      if (app != null) {
        await insertActivityEvent(this.partnerId, app, "open", tsMs);
        this.prevExe = app;
        this.prevStartedAt = tsMs;
      }
      // else: idle → idle, nothing to record.
    }
  }

  /** Reset the derived state (called on unpair/reconnect so a fresh session
   *  starts a new history chain without mis-deriving a `close`). */
  reset(): void {
    this.prevExe = null;
    this.prevStartedAt = null;
  }
}
