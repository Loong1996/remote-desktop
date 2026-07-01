import { expect, test } from "vitest";
import { parseInputEvent } from "../src/input.js";

test("parses mouse move with relative coords", () => {
  expect(parseInputEvent({ t: "mmove", x: 0.5, y: 0.25 })).toEqual({ t: "mmove", x: 0.5, y: 0.25 });
});
test("parses mouse button", () => {
  expect(parseInputEvent({ t: "mdown", button: "left" }).t).toBe("mdown");
});
test("parses key event", () => {
  expect(parseInputEvent({ t: "kdown", code: "KeyA" })).toEqual({ t: "kdown", code: "KeyA" });
});
test("rejects out-of-range coord", () => {
  expect(() => parseInputEvent({ t: "mmove", x: 1.5, y: 0 })).toThrow();
});
test("rejects unknown button", () => {
  expect(() => parseInputEvent({ t: "mdown", button: "middle-left" })).toThrow();
});
