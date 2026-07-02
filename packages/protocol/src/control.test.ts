import { describe, it, expect } from "vitest";
import { parseControlMessage, CLIP_MAX_BYTES } from "./control.js";

describe("parseControlMessage", () => {
  it("parses each variant", () => {
    expect(parseControlMessage({ t: "clip-set", text: "hi" })).toEqual({ t: "clip-set", text: "hi" });
    expect(parseControlMessage({ t: "clip-request" })).toEqual({ t: "clip-request" });
    expect(parseControlMessage({ t: "clip-mode", mode: "both" })).toEqual({ t: "clip-mode", mode: "both" });
    expect(parseControlMessage({ t: "quality", bitrateBps: 3_000_000 })).toEqual({ t: "quality", bitrateBps: 3_000_000 });
  });

  it("rejects malformed messages", () => {
    expect(() => parseControlMessage(null)).toThrow();
    expect(() => parseControlMessage({ t: "nope" })).toThrow();
    expect(() => parseControlMessage({ t: "clip-set", text: 5 })).toThrow();
    expect(() => parseControlMessage({ t: "clip-set", text: "x".repeat(CLIP_MAX_BYTES + 1) })).toThrow();
    expect(() => parseControlMessage({ t: "clip-mode", mode: "sideways" })).toThrow();
    expect(() => parseControlMessage({ t: "quality", bitrateBps: 10 })).toThrow();
    expect(() => parseControlMessage({ t: "quality", bitrateBps: 99_000_000 })).toThrow();
  });
});
