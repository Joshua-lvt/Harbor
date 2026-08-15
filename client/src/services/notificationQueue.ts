/**
 * Harbor notification queue (Feature 5).
 *
 * Quick-messages are E2E-encrypted notifications that arrive through the normal
 * `chat.enc` path and are detected in useChat (see the `harbor_notify` inner
 * kind). They are **notification-only**: never stored in the chat log, never
 * rendered as a chat bubble. This module owns the transient queue both surfaces
 * (the in-app Toaster when the main window is focused, and the separate
 * always-on-top `notif-overlay` Tauri window when it isn't) draw from.
 *
 * The queue is a process singleton living in the MAIN window's JS realm. The
 * `notif-overlay` is a separate Tauri window = a separate JS realm, so it cannot
 * import this module and share state; instead `useHarborNotifications` vents
 * each enqueued notification to the overlay via a Tauri event
 * (`services/notifOverlay.ts:emitNotification`), exactly like the widget gets its
 * snapshot from the main window over an event. The overlay never opens a socket
 * (one-socket-per-device rule).
 *
 * Grouping: identical presets (same `presetId`) arriving within 5s collapse into
 * a single card with an incremented `repeatCount` ("×3"). Display: one at a time,
 * 5s each, 500ms gap (the hook drives the slot rotation; this module only counts
 * + notifies listeners).
 */
export interface HarborNotification {
  /** Stable id (the chat envelope id) so dismiss is idempotent and grouping
   *  targets the live notification rather than a duplicate. */
  id: string;
  partnerName: string;
  partnerAvatar: string | null;
  text: string;
  presetId?: string;
  timestamp: number;
  /** How many identical presets collapsed into this card (1 = single). */
  repeatCount: number;
}

/** Window during which an identical `presetId` collapses into the live card. */
const GROUP_WINDOW_MS = 5000;

type ChangeHandler = (queue: HarborNotification[]) => void;
type NotificationHandler = (n: HarborNotification) => void;

class NotificationQueue {
  private items: HarborNotification[] = [];
  private changeHandlers = new Set<ChangeHandler>();
  private notifHandlers = new Set<NotificationHandler>();

  /** Enqueue a quick-message. If the front card shares the same `presetId` and
   *  was enqueued within GROUP_WINDOW_MS, collapse into it (increment
   *  `repeatCount`) instead of pushing a new card. Always fires
   *  `onNotification` (the surface re-evaluates the slot) + `onQueueChange`. */
  enqueue(n: Omit<HarborNotification, "repeatCount"> & { repeatCount?: number }): void {
    const now = Date.now();
    const front = this.items[0];
    if (
      front &&
      n.presetId &&
      front.presetId === n.presetId &&
      now - front.timestamp <= GROUP_WINDOW_MS
    ) {
      // Collapse into the live card: bump its count + refresh its timestamp so
      // the 5s display window resets, and surface the update immediately.
      front.repeatCount += 1;
      front.timestamp = now;
      front.text = n.text; // keep the latest text in case it drifted
      this.emitChange();
      this.emitNotification(front);
      return;
    }
    const item: HarborNotification = { ...n, repeatCount: n.repeatCount ?? 1 };
    this.items.push(item);
    this.emitChange();
    this.emitNotification(item);
  }

  /** Remove a notification by id (dismiss / "Ignorar"). */
  dismiss(id: string): void {
    const before = this.items.length;
    this.items = this.items.filter((x) => x.id !== id);
    if (this.items.length !== before) this.emitChange();
  }

  /** Drop the front card — used by the slot timer after its display window ends. */
  shiftFront(): void {
    if (!this.items.length) return;
    this.items.shift();
    this.emitChange();
  }

  getQueue(): HarborNotification[] {
    return this.items;
  }

  isEmpty(): boolean {
    return this.items.length === 0;
  }

  onNotification(h: NotificationHandler): () => void {
    this.notifHandlers.add(h);
    return () => this.notifHandlers.delete(h);
  }

  onQueueChange(h: ChangeHandler): () => void {
    this.changeHandlers.add(h);
    h(this.items);
    return () => this.changeHandlers.delete(h);
  }

  private emitChange(): void {
    for (const h of this.changeHandlers) h(this.items);
  }
  private emitNotification(n: HarborNotification): void {
    for (const h of this.notifHandlers) h(n);
  }
}

export const notificationQueue = new NotificationQueue();

/** Display duration + gap (the hook drives the slot rotation off these). */
export const NOTIF_DISPLAY_MS = 5000;
export const NOTIF_GAP_MS = 500;
