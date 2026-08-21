/**
 * clockOffset — shared clock-offset correction for server timestamps.
 *
 * The relay stamps events with `ts` in SECONDS of ITS wall clock; our own
 * outgoing messages are stamped with `Date.now()` (ms) of OUR clock. If the two
 * clocks differ, a raw server `ts` would place received events "in the past"
 * (or future) relative to our own. We sample the offset from the first received
 * event of a session and reuse it, so received timestamps are corrected to our
 * clock while preserving the server's relative ordering.
 *
 * Shared by messages.ts (chat) and notify.ts (presence) so both write to the
 * notification history on the SAME clock — otherwise messages and presence
 * events would be out of relative order in the Notifications tab.
 */
import { asMs } from "./localDb";

let clockOffsetMs: number | null = null;

/** Sample the offset from a server timestamp (seconds). The first sample of a
 *  session sets the estimate immediately (fixing the "messages in the past"
 *  bug); every later sample revalidates it via an exponential moving average so
 *  a single contaminated sample (a latency spike / burst) only nudges the
 *  estimate slightly instead of biasing every timestamp forever. Call
 *  `resetClockOffset` to drop the estimate (e.g. on unpair). */
export function sampleClockOffset(serverTs: number): void {
  const sample = Date.now() - asMs(serverTs);
  if (clockOffsetMs === null) {
    clockOffsetMs = sample;
  } else {
    // EMA weight 0.1: ~20 messages to converge, smooths per-message latency.
    clockOffsetMs = clockOffsetMs * 0.9 + sample * 0.1;
  }
}

/** Drop the sampled offset so the next event re-samples it. */
export function resetClockOffset(): void {
  clockOffsetMs = null;
}

/** Correct a server timestamp (seconds) to our local clock (ms). Falls back to
 *  the raw value if no offset has been sampled yet. */
export function correctServerTs(ts: number): number {
  const serverMs = asMs(ts);
  return clockOffsetMs === null ? serverMs : serverMs + clockOffsetMs;
}
