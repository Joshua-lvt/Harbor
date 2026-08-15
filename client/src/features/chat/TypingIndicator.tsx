/**
 * Animated three-dot "partner is typing" indicator. Renders nothing when not
 * visible, so the header layout doesn't reserve space for it. Staggered via
 * the global `typingDot` keyframe (style.css).
 */
export function TypingIndicator({ visible }: { visible: boolean }) {
  if (!visible) return null;
  return (
    <span className="inline-flex items-center gap-0.5 ml-1 align-middle">
      {[0, 1, 2].map((i) => (
        <span
          key={i}
          className="w-1 h-1 rounded-full bg-harbor-sea inline-block"
          style={{ animation: `typingDot 1s ease-in-out ${i * 0.15}s infinite` }}
        />
      ))}
    </span>
  );
}

export default TypingIndicator;
