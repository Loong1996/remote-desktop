import { describe, it, expect, test } from "vitest";
import { parseSignalingMessage, parseInputEvent } from "@rd/protocol";
import {
  deriveWsUrl,
  buildConnect,
  buildOffer,
  buildIce,
  mouseCoords,
  mouseButtonName,
} from "./rtc.js";

describe("deriveWsUrl", () => {
  it("maps http:// to ws:// and appends the token query", () => {
    expect(deriveWsUrl("http://127.0.0.1:8080", "jwt-123")).toBe(
      "ws://127.0.0.1:8080/?token=jwt-123",
    );
  });

  it("maps https:// to wss://", () => {
    expect(deriveWsUrl("https://relay.example.com", "abc")).toBe(
      "wss://relay.example.com/?token=abc",
    );
  });

  it("preserves an explicit path and port", () => {
    expect(deriveWsUrl("https://host:9443/signal", "t0k")).toBe(
      "wss://host:9443/signal?token=t0k",
    );
  });

  it("url-encodes the token (query form-encoding: space→+, slash/plus escaped)", () => {
    expect(deriveWsUrl("http://h", "a b/c+d")).toBe("ws://h/?token=a+b%2Fc%2Bd");
  });
});

describe("buildConnect", () => {
  it("produces {type:'connect',deviceId} accepted by parseSignalingMessage", () => {
    const msg = buildConnect("device-42");
    expect(msg).toEqual({ type: "connect", deviceId: "device-42" });
    expect(parseSignalingMessage(msg)).toEqual(msg);
  });
});

describe("buildOffer", () => {
  it("wraps an SDP offer into a {type:'sdp',sessionId,sdp:{type:'offer',sdp}} message", () => {
    const offer: RTCSessionDescriptionInit = { type: "offer", sdp: "v=0\r\n" };
    const msg = buildOffer("sess-1", offer);
    expect(msg).toEqual({
      type: "sdp",
      sessionId: "sess-1",
      sdp: { type: "offer", sdp: "v=0\r\n" },
    });
    expect(parseSignalingMessage(msg)).toEqual(msg);
  });
});

describe("buildIce", () => {
  it("produces {type:'ice',sessionId,candidate} accepted by parseSignalingMessage", () => {
    const candidate = { candidate: "candidate:1 1 udp ...", sdpMid: "0", sdpMLineIndex: 0 };
    const msg = buildIce("sess-1", candidate);
    expect(msg).toEqual({ type: "ice", sessionId: "sess-1", candidate });
    expect(parseSignalingMessage(msg)).toEqual(msg);
  });
});

test("mouseCoords produces clamped [0,1] relative coords", () => {
  const rect = { left: 100, top: 50, width: 800, height: 600 };
  expect(mouseCoords(500, 350, rect)).toEqual({ x: 0.5, y: 0.5 });
  // out-of-bounds clamps into range
  expect(mouseCoords(0, 0, rect)).toEqual({ x: 0, y: 0 });
  expect(mouseCoords(2000, 2000, rect)).toEqual({ x: 1, y: 1 });
});

test("mouseButtonName maps DOM button ids", () => {
  expect(mouseButtonName(0)).toBe("left");
  expect(mouseButtonName(1)).toBe("middle");
  expect(mouseButtonName(2)).toBe("right");
  expect(mouseButtonName(3)).toBeNull();
});

test("encoded events pass the protocol validator", () => {
  const { x, y } = mouseCoords(500, 350, { left: 100, top: 50, width: 800, height: 600 });
  expect(parseInputEvent({ t: "mmove", x, y })).toEqual({ t: "mmove", x: 0.5, y: 0.5 });
  expect(parseInputEvent({ t: "mdown", button: mouseButtonName(0) })).toEqual({
    t: "mdown",
    button: "left",
  });
});
