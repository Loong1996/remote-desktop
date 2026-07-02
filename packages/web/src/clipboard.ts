import type { ClipMode } from "@rd/protocol";
export type { ClipMode };

export const CLIP_MAX_BYTES = 262144;

/** The text to send given the current clipboard + last-known value, or null to
 *  skip (empty, unchanged, or over the UTF-8 byte cap — never truncate). */
export function clipboardToSend(
  current: string,
  lastKnown: string,
  capBytes = CLIP_MAX_BYTES,
): string | null {
  if (current.length === 0 || current === lastKnown) return null;
  if (new TextEncoder().encode(current).length > capBytes) return null;
  return current;
}
