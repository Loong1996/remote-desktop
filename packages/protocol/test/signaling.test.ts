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

test("parses an agent-online message", () => {
  const msg = parseSignalingMessage({ type: "agent-online", token: "tok-123" });
  expect(msg).toEqual({ type: "agent-online", token: "tok-123" });
});

test("parses a peer-left message", () => {
  const msg = parseSignalingMessage({ type: "peer-left", sessionId: "s1" });
  expect(msg).toEqual({ type: "peer-left", sessionId: "s1" });
});

test("parses an ice message", () => {
  const msg = parseSignalingMessage({ type: "ice", sessionId: "s1", candidate: { foo: 1 } });
  expect(msg.type).toBe("ice");
  expect(msg.sessionId).toBe("s1");
});

test("rejects sdp with invalid inner type", () => {
  expect(() => parseSignalingMessage({ type: "sdp", sessionId: "s1", sdp: { type: "bogus", sdp: "v=0" } })).toThrow();
});

test("rejects agent-online with empty token", () => {
  expect(() => parseSignalingMessage({ type: "agent-online", token: "" })).toThrow();
});

test("rejects connect with non-string deviceId", () => {
  expect(() => parseSignalingMessage({ type: "connect", deviceId: 123 })).toThrow();
});
