import { describe, it, expect } from "vitest";
import { comboEvents } from "./combos.js";

describe("comboEvents", () => {
  it("presses in order then releases in reverse", () => {
    expect(comboEvents(["MetaLeft", "KeyC"])).toEqual([
      { t: "kdown", code: "MetaLeft" },
      { t: "kdown", code: "KeyC" },
      { t: "kup", code: "KeyC" },
      { t: "kup", code: "MetaLeft" },
    ]);
  });
  it("handles a single key", () => {
    expect(comboEvents(["Escape"])).toEqual([
      { t: "kdown", code: "Escape" },
      { t: "kup", code: "Escape" },
    ]);
  });
  it("returns empty for empty input", () => {
    expect(comboEvents([])).toEqual([]);
  });
});
