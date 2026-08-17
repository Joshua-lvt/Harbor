/**
 * Harbor mascot — a small, rounded, friendly stylized shark.
 * Minimalist, ocean palette, Windows 11 soft look. Used in pairing, Home
 * and empty states.
 */
import type { CSSProperties } from "react";

export function SharkMascot({
  className,
  style,
}: {
  className?: string;
  style?: CSSProperties;
}) {
  return (
    <svg viewBox="0 0 120 120" className={className} style={style} fill="none" xmlns="http://www.w3.org/2000/svg" aria-hidden>
      <defs>
        <linearGradient id="harborShark" x1="0" y1="0" x2="0" y2="1">
          <stop offset="0%" stopColor="#90CDF4" />
          <stop offset="100%" stopColor="#63B3ED" />
        </linearGradient>
      </defs>
      {/* Body */}
      <path
        d="M14 64c22-20 56-24 80-14 14 5 16 18 4 24-10 5-26 6-42 3-2 14-12 22-12 22s4-16 2-26c-14-4-26-5-32-9z"
        fill="url(#harborShark)"
        stroke="#2B6CB0"
        strokeWidth="3"
        strokeLinejoin="round"
      />
      {/* Belly */}
      <path
        d="M24 70c18 4 38 4 56-2-6 8-18 12-30 11-10 0-20-4-26-9z"
        fill="#F7FAFC"
        opacity="0.85"
      />
      {/* Tail */}
      <path d="M14 64c-4-10-2-20 4-28-2 10 0 20 4 28z" fill="#63B3ED" stroke="#2B6CB0" strokeWidth="3" strokeLinejoin="round" />
      {/* Dorsal */}
      <path d="M52 50c2-12 10-18 18-18-6 6-8 12-6 20z" fill="#63B3ED" stroke="#2B6CB0" strokeWidth="3" strokeLinejoin="round" />
      {/* Eye */}
      <circle cx="86" cy="62" r="3.6" fill="#1A365D" />
      <circle cx="87.2" cy="60.8" r="1.1" fill="#F7FAFC" />
      {/* Smile */}
      <path d="M74 70c4 4 10 4 14 1" stroke="#1A365D" strokeWidth="2.4" strokeLinecap="round" fill="none" />
    </svg>
  );
}
