export type MouseButton = "left" | "right" | "middle";

export interface MouseMove { t: "mmove"; x: number; y: number; }      // x,y ∈ [0,1]
export interface MouseDown { t: "mdown"; button: MouseButton; }
export interface MouseUp   { t: "mup";   button: MouseButton; }
export interface Wheel     { t: "wheel"; dx: number; dy: number; }
export interface KeyDown   { t: "kdown"; code: string; }              // KeyboardEvent.code
export interface KeyUp     { t: "kup";   code: string; }

export type InputEvent = MouseMove | MouseDown | MouseUp | Wheel | KeyDown | KeyUp;

const BUTTONS = new Set(["left", "right", "middle"]);

function isObj(v: unknown): v is Record<string, unknown> {
  return typeof v === "object" && v !== null;
}
function num(v: unknown, field: string): number {
  if (typeof v !== "number" || Number.isNaN(v)) throw new Error(`invalid number: ${field}`);
  return v;
}
function coord(v: unknown, field: string): number {
  const n = num(v, field);
  if (n < 0 || n > 1) throw new Error(`coord out of range [0,1]: ${field}`);
  return n;
}
function button(v: unknown): MouseButton {
  if (typeof v !== "string" || !BUTTONS.has(v)) throw new Error("invalid button");
  return v as MouseButton;
}
function str(v: unknown, field: string): string {
  if (typeof v !== "string" || v.length === 0) throw new Error(`invalid field: ${field}`);
  return v;
}

export function parseInputEvent(raw: unknown): InputEvent {
  if (!isObj(raw)) throw new Error("event must be an object");
  switch (raw.t) {
    case "mmove": return { t: "mmove", x: coord(raw.x, "x"), y: coord(raw.y, "y") };
    case "mdown": return { t: "mdown", button: button(raw.button) };
    case "mup":   return { t: "mup",   button: button(raw.button) };
    case "wheel": return { t: "wheel", dx: num(raw.dx, "dx"), dy: num(raw.dy, "dy") };
    case "kdown": return { t: "kdown", code: str(raw.code, "code") };
    case "kup":   return { t: "kup",   code: str(raw.code, "code") };
    default: throw new Error(`unknown input type: ${String(raw.t)}`);
  }
}
