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

  it("caps clip-set by UTF-8 byte length, not UTF-16 units", () => {
    // "€" is 3 UTF-8 bytes; 90000 chars = 270000 bytes > CLIP_MAX_BYTES,
    // yet .length (90000) is far below the cap — must still be rejected.
    const euros = "€".repeat(90000);
    expect(euros.length).toBeLessThan(262144); // discriminates byte-based from length-based
    expect(() => parseControlMessage({ t: "clip-set", text: euros })).toThrow();
  });

  describe("resolution", () => {
    it("parses each valid preset", () => {
      for (const preset of ["sd", "hd", "native"] as const) {
        expect(parseControlMessage({ t: "resolution", preset })).toEqual({ t: "resolution", preset });
      }
    });

    it("rejects unknown or missing presets", () => {
      expect(() => parseControlMessage({ t: "resolution", preset: "8k" })).toThrow(/resolution\.preset/);
      expect(() => parseControlMessage({ t: "resolution" })).toThrow(/resolution\.preset/);
      expect(() => parseControlMessage({ t: "resolution", preset: 2 })).toThrow(/resolution\.preset/);
    });
  });
});
