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
test("accepts boundary 0: coord x=0, y=0", () => {
  expect(parseInputEvent({ t: "mmove", x: 0, y: 0 })).toEqual({ t: "mmove", x: 0, y: 0 });
});
test("accepts boundary 1: coord x=1, y=1", () => {
  expect(parseInputEvent({ t: "mmove", x: 1, y: 1 })).toEqual({ t: "mmove", x: 1, y: 1 });
});
test("rejects negative coord", () => {
  expect(() => parseInputEvent({ t: "mmove", x: -0.1, y: 0 })).toThrow();
});
test("rejects NaN coord", () => {
  expect(() => parseInputEvent({ t: "mmove", x: NaN, y: 0 })).toThrow();
});
test("parses wheel event", () => {
  expect(parseInputEvent({ t: "wheel", dx: -3, dy: 10 })).toEqual({ t: "wheel", dx: -3, dy: 10 });
});
test("parses key up event", () => {
  expect(parseInputEvent({ t: "kup", code: "Escape" })).toEqual({ t: "kup", code: "Escape" });
});
