/**
 * OceanBackground — the decorative layer behind the Home content.
 *
 * A soft vertical gradient (#F8FBFF → #EEF4FF, matching the `.window-main`
 * surface) plus a few very subtle translucent bubbles, blurred waves in the
 * bottom corner, and a faint top glow. Everything is pointer-events-none and
 * sits behind at -z-10, so it never interferes with input or layout.
 *
 * Kept deliberately understated — calm, not childish. The whole layer is fixed
 * effort (no runtime cost, no state) so the Home feels like a space, not a
 * panel.
 */
export function OceanBackground() {
  return (
    <div aria-hidden className="pointer-events-none absolute inset-0 -z-10 overflow-hidden">
      {/* Top soft blue glow */}
      <div className="absolute -top-24 left-1/2 h-48 w-[36rem] -translate-x-1/2 rounded-full bg-harbor-sea/15 blur-3xl" />

      {/* Translucent bubbles — sparse, large, softly blurred. Kept on the
          warm side (white at low alpha, or ice) — they read as soft highlights
          on the light bg and as faint cool glows on the dark ocean bg. */}
      <div className="absolute left-[12%] top-[18%] h-24 w-24 rounded-full bg-white/20 blur-2xl dark:bg-harbor-ice/10" />
      <div className="absolute right-[16%] top-[30%] h-16 w-16 rounded-full bg-harbor-ice/20 blur-2xl dark:bg-harbor-ice/10" />
      <div className="absolute left-[42%] top-[14%] h-10 w-10 rounded-full bg-white/25 blur-xl dark:bg-harbor-ice/10" />

      {/* Blurred waves along the bottom — wide, flat, low-opacity ocean hint */}
      <div className="absolute -bottom-16 -left-10 h-40 w-[28rem] rounded-[50%] bg-harbor-sea/10 blur-3xl" />
      <div className="absolute -bottom-20 right-0 h-44 w-[32rem] rounded-[50%] bg-harbor-ice/10 blur-3xl" />
    </div>
  );
}

export default OceanBackground;
