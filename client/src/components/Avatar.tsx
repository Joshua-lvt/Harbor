/**
 * Avatar — a circular profile photo with a graceful fallback to the shark
 * mascot when no photo is set (or a broken data URL fails to load). Used
 * across Home (sidebar, partner hero), the call panel, the widget, and the
 * chat header.
 *
 * `ring` controls a blue ring around the avatar, used by the call panel to
 * show who is speaking:
 *   - "speaking" — an animated pulsing blue ring (see `speakingRing` keyframe
 *     in style.css), shown around whoever's mic is hot right now.
 *   - "idle"     — a static subtle ring, shown for people in the call but not
 *     talking, so the two call avatars read as a pair even when both are muted.
 *   - undefined  — no ring (the normal, non-call display).
 *
 * Pure presentation apart from a one-bit `broken` flag tracking failed <img>
 * loads (a malformed data URL shouldn't leave a hole — fall back to the mascot).
 */
import { useState } from "react";
import { SharkMascot } from "../assets/shark";

type RingState = "speaking" | "idle";

export function Avatar({
  src,
  alt,
  size = 40,
  ring,
  className = "",
}: {
  src?: string | null;
  alt?: string;
  size?: number;
  ring?: RingState;
  /** Extra classes for the outer wrapper (e.g. positioning). */
  className?: string;
}) {
  const [broken, setBroken] = useState(false);
  const showImg = !!src && !broken;

  // The speaking ring is box-shadow ONLY — the `speaking-ring` keyframe (in
  // style.css) owns the whole shadow so it animates cleanly. Tailwind's `ring-*`
  // utility would set its own `--tw-ring-*` box-shadow and fight the keyframe,
  // so we deliberately don't combine them. "idle" is a static subtle ring.
  // The large call-panel orbs (≥56px) use the brighter, larger `*-lg` ring so
  // the speaking indicator stays obvious; smaller avatars keep the subtle ring.
  const ringClass =
    ring === "speaking"
      ? size >= 56
        ? "speaking-ring-lg"
        : "speaking-ring"
      : ring === "idle"
        ? size >= 56
          ? "ring-2 ring-harbor-sea/50"
          : "ring-2 ring-harbor-sea/40"
        : "";

  return (
    <div
      className={`relative flex shrink-0 items-center justify-center overflow-hidden rounded-full bg-harbor-sky/30 ${ringClass} ${className}`}
      style={{ width: size, height: size }}
    >
      {showImg ? (
        <img
          src={src}
          alt={alt ?? "Avatar"}
          className="h-full w-full object-cover"
          draggable={false}
          onError={() => setBroken(true)}
        />
      ) : (
        <SharkMascot className="relative" style={{ width: size * 0.62, height: size * 0.62 }} />
      )}
    </div>
  );
}

export default Avatar;
