import { describe, it, expect } from "vitest";
import { clipboardToSend } from "./clipboard.js";

describe("clipboardToSend", () => {
  it("skips unchanged, empty, and oversized; returns changed text", () => {
    expect(clipboardToSend("a", "a")).toBeNull(); // unchanged
    expect(clipboardToSend("", "a")).toBeNull(); // empty
    expect(clipboardToSend("b", "a")).toBe("b"); // changed
    expect(clipboardToSend("x".repeat(11), "a", 10)).toBeNull(); // over cap (11 bytes > 10)
  });

  it("caps by UTF-8 byte length, not UTF-16 code units", () => {
    // "€" = 3 UTF-8 bytes: 4 chars = 12 bytes > cap 10, though .length (4) < 10.
    expect(clipboardToSend("€€€€", "prev", 10)).toBeNull();
  });
});
