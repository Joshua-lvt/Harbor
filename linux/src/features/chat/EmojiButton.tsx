/**
 * A small, hand-rolled emoji picker (no npm dep). A curated ocean + common
 * set opens in a popover; clicking one appends it to the draft. An overlay
 * behind the popover closes it on outside click.
 */
import { useState } from "react";

const EMOJIS = [
  "💙", "🌊", "🐬", "🦈", "❤️", "😂", "😍", "🥰", "😴", "🤔",
  "🙏", "🎉", "🔥", "⭐", "✨", "🥺", "😊", "😘", "🤗", "💕",
  "😇", "😎", "😟", "🧊", "☕", "🌙", "☀️", "⛱️", "🍪", "🎁",
];

export function EmojiButton({ onPick }: { onPick: (emoji: string) => void }) {
  const [open, setOpen] = useState(false);
  return (
    <div className="relative">
      <button
        type="button"
        onClick={() => setOpen((o) => !o)}
        className="text-lg text-harbor-deep/70 hover:text-harbor-deep px-1"
        aria-label="Emoji"
      >
        😊
      </button>
      {open && (
        <>
          <div className="fixed inset-0 z-10" onClick={() => setOpen(false)} />
          <div className="absolute bottom-9 left-0 z-20 grid grid-cols-6 gap-1 bg-harbor-surface-strong border border-harbor-sky rounded-xl shadow-lg p-2 w-60">
            {EMOJIS.map((e) => (
              <button
                key={e}
                type="button"
                className="text-xl hover:bg-harbor-sky/40 rounded-md p-1"
                onClick={() => {
                  onPick(e);
                  setOpen(false);
                }}
              >
                {e}
              </button>
            ))}
          </div>
        </>
      )}
    </div>
  );
}

export default EmojiButton;
