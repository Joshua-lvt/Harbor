/** Stable room identifiers and participant checks for a pair call. */

export function roomIdForPair(deviceA: string, deviceB: string): string {
  const a = deviceA.trim();
  const b = deviceB.trim();
  if (!a || !b || a === b) throw new Error("a call room requires two distinct devices");
  const [first, second] = [a, b].sort();
  return `pair:${encodeURIComponent(first)}:${encodeURIComponent(second)}`;
}

export function isRoomParticipant(roomId: string, deviceId: string): boolean {
  const id = deviceId.trim();
  if (!id || !roomId.startsWith("pair:")) return false;
  const parts = roomId.slice("pair:".length).split(":");
  if (parts.length !== 2) return false;
  return parts.some((part) => {
    try {
      return decodeURIComponent(part) === id;
    } catch {
      return false;
    }
  });
}

export function otherRoomParticipant(roomId: string, deviceId: string): string | null {
  const id = deviceId.trim();
  if (!id || !roomId.startsWith("pair:")) return null;
  const parts = roomId.slice("pair:".length).split(":");
  if (parts.length !== 2) return null;
  try {
    const ids = parts.map((part) => decodeURIComponent(part));
    if (!ids.includes(id)) return null;
    return ids.find((candidate) => candidate !== id) ?? null;
  } catch {
    return null;
  }
}
