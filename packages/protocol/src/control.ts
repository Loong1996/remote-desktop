export type ClipMode = "off" | "oneway" | "both";

export type ResolutionPreset = "sd" | "hd" | "native";

export interface ClipSet { t: "clip-set"; text: string; }
export interface ClipRequest { t: "clip-request"; }
export interface ClipModeMsg { t: "clip-mode"; mode: ClipMode; }
export interface Quality { t: "quality"; bitrateBps: number; }
export interface Resolution { t: "resolution"; preset: ResolutionPreset; }

export type ControlMessage = ClipSet | ClipRequest | ClipModeMsg | Quality | Resolution;

// 256 KB cap on clip-set text, measured in UTF-8 bytes on BOTH sides (web via
// TextEncoder, agent via String::len) so the two ends accept/reject identically.
export const CLIP_MAX_BYTES = 262144;
export const QUALITY_MIN_BPS = 250_000;
export const QUALITY_MAX_BPS = 20_000_000;

const CLIP_MODES = new Set<ClipMode>(["off", "oneway", "both"]);
const RESOLUTION_PRESETS = new Set<ResolutionPreset>(["sd", "hd", "native"]);

function isObj(v: unknown): v is Record<string, unknown> {
  return typeof v === "object" && v !== null;
}

export function parseControlMessage(raw: unknown): ControlMessage {
  if (!isObj(raw)) throw new Error("control message must be an object");
  switch (raw.t) {
    case "clip-set": {
      if (typeof raw.text !== "string") throw new Error("clip-set.text must be a string");
      if (new TextEncoder().encode(raw.text).length > CLIP_MAX_BYTES) throw new Error("clip-set.text too large");
      return { t: "clip-set", text: raw.text };
    }
    case "clip-request":
      return { t: "clip-request" };
    case "clip-mode": {
      if (typeof raw.mode !== "string" || !CLIP_MODES.has(raw.mode as ClipMode))
        throw new Error("invalid clip-mode.mode");
      return { t: "clip-mode", mode: raw.mode as ClipMode };
    }
    case "quality": {
      const n = raw.bitrateBps;
      if (typeof n !== "number" || Number.isNaN(n) || n < QUALITY_MIN_BPS || n > QUALITY_MAX_BPS)
        throw new Error("invalid quality.bitrateBps");
      return { t: "quality", bitrateBps: n };
    }
    case "resolution": {
      if (typeof raw.preset !== "string" || !RESOLUTION_PRESETS.has(raw.preset as ResolutionPreset))
        throw new Error("invalid resolution.preset");
      return { t: "resolution", preset: raw.preset as ResolutionPreset };
    }
    default:
      throw new Error(`unknown control type: ${String(raw.t)}`);
  }
}
