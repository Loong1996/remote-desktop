import type { InputEvent } from "@rd/protocol";

export interface Combo {
  label: string;
  codes: string[]; // KeyboardEvent.code values, held together
}

/** macOS chords the browser would otherwise swallow. */
export const COMBOS: Combo[] = [
  { label: "Spotlight", codes: ["MetaLeft", "Space"] },
  { label: "App Switcher", codes: ["MetaLeft", "Tab"] },
  { label: "Mission Control", codes: ["ControlLeft", "ArrowUp"] },
  { label: "Screenshot", codes: ["MetaLeft", "ShiftLeft", "Digit4"] },
  { label: "Copy", codes: ["MetaLeft", "KeyC"] },
  { label: "Paste", codes: ["MetaLeft", "KeyV"] },
  { label: "Close Window", codes: ["MetaLeft", "KeyW"] },
  { label: "Quit App", codes: ["MetaLeft", "KeyQ"] },
  { label: "Esc", codes: ["Escape"] },
];

/** Press all `codes` in order (kdown), then release in reverse order (kup). */
export function comboEvents(codes: string[]): InputEvent[] {
  const out: InputEvent[] = [];
  for (const code of codes) out.push({ t: "kdown", code });
  for (let i = codes.length - 1; i >= 0; i--) out.push({ t: "kup", code: codes[i] });
  return out;
}
