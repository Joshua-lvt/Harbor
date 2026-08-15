/**
 * A single chat bubble. Owns its own entry animation (`bubblePop`) and renders
 * emoji-only messages larger, like Discord/messaging apps. Image attachments
 * (a compressed JPEG data URL on `message.image`) render above the text
 * caption, sized to fit within the bubble's max width. Mine → deep blue
 * right-aligned; partner → white left-aligned. Status below mine only.
 */
import type { StoredMessage } from "../../lib/types";

function isEmojiOnly(text: string): boolean {
  const t = text.trim();
  if (!t) return false;
  // Extended_Pictographic covers most emoji; also allow ZWJ / variation
  // selectors / whitespace between emoji.
  return /^(\p{Extended_Pictographic}|‍|️|\s)+$/u.test(t);
}

export function ChatBubble({ message }: { message: StoredMessage }) {
  const time = new Date(message.created_at).toLocaleTimeString([], {
    hour: "2-digit",
    minute: "2-digit",
  });
  const mine = message.from_me;
  const hasImage = !!message.image;
  const text = message.text;
  const hasText = text.trim().length > 0;
  // Giant-emoji styling only applies to a *pure* text emoji message — an image
  // with an emoji caption still uses the normal bubble so it stays predictable.
  const emojiOnly = !hasImage && isEmojiOnly(text);

  return (
    <div
      className={`bubble-enter max-w-[78%] flex flex-col ${
        mine ? "self-end items-end" : "self-start items-start"
      }`}
    >
      {hasImage && (
        <img
          src={message.image!}
          alt={hasText ? text : "Imagem"}
          loading="lazy"
          className={`rounded-2xl shadow-sm max-h-72 max-w-full ${mine ? "rounded-br-md" : "rounded-bl-md"} ${
            hasText ? "mb-1" : ""
          }`}
        />
      )}
      {hasText && (
        <div
          className={
            emojiOnly
              ? "text-3xl leading-tight"
              : `rounded-2xl px-3 py-2 text-sm shadow-sm ${
                  mine
                    ? "bg-harbor-deep text-white rounded-br-md"
                    : "bg-harbor-surface text-harbor-ink rounded-bl-md"
                }`
          }
        >
          {text}
        </div>
      )}
      {!emojiOnly && (
        <span className="text-[10px] text-harbor-ink/50 mt-0.5">
          {time}
          {mine && message.status === "sending" && " · enviando"}
          {mine && message.status === "delivered" && " · ✓"}
        </span>
      )}
    </div>
  );
}

export default ChatBubble;
