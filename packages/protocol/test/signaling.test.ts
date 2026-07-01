import { expect, test } from "vitest";
import { parseSignalingMessage } from "../src/signaling.js";

test("parses a connect message", () => {
  const msg = parseSignalingMessage({ type: "connect", deviceId: "dev-1" });
  expect(msg).toEqual({ type: "connect", deviceId: "dev-1" });
});

test("parses an sdp relay message", () => {
  const msg = parseSignalingMessage({
    type: "sdp", sessionId: "s1", sdp: { type: "offer", sdp: "v=0..." },
  });
  expect(msg.type).toBe("sdp");
});

test("rejects unknown type", () => {
  expect(() => parseSignalingMessage({ type: "nope" })).toThrow();
});

test("rejects connect without deviceId", () => {
  expect(() => parseSignalingMessage({ type: "connect" })).toThrow();
});
