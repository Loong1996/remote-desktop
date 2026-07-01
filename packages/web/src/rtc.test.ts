import { describe, it, expect, test } from "vitest";
import { parseSignalingMessage, parseInputEvent } from "@rd/protocol";
import {
  deriveWsUrl,
  buildConnect,
  buildOffer,
  buildIce,
  mouseCoords,
  mouseButtonName,
  streamFromTrackEvent,
  contentRect,
  releaseEvents,
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

test("streamFromTrackEvent prefers event.streams[0]", () => {
  const stream = { id: "s1" } as unknown as MediaStream;
  const ev = { streams: [stream], track: { kind: "video" } } as unknown as RTCTrackEvent;
  expect(streamFromTrackEvent(ev)).toBe(stream);
});

test("streamFromTrackEvent falls back to a new stream from the track", () => {
  const track = { kind: "video" } as unknown as MediaStreamTrack;
  const ev = { streams: [], track } as unknown as RTCTrackEvent;
  const s = streamFromTrackEvent(ev, (t) => ({ tracks: [t] }) as unknown as MediaStream);
  expect((s as unknown as { tracks: MediaStreamTrack[] }).tracks[0]).toBe(track);
});

test("contentRect: wide video in a square element letterboxes top/bottom", () => {
  const r = contentRect({ width: 400, height: 400 }, 1600, 900);
  expect(r.width).toBe(400);
  expect(r.height).toBe(225);
  expect(r.left).toBe(0);
  expect(r.top).toBe(87.5);
});

test("contentRect: tall video in a wide element pillarboxes left/right", () => {
  const r = contentRect({ width: 400, height: 200 }, 100, 200);
  expect(r.height).toBe(200);
  expect(r.width).toBe(100);
  expect(r.top).toBe(0);
  expect(r.left).toBe(150);
});

test("contentRect: no stream falls back to the element box", () => {
  expect(contentRect({ width: 320, height: 240 }, 0, 0)).toEqual({
    left: 0, top: 0, width: 320, height: 240,
  });
});

test("releaseEvents produces kup/mup for held keys and buttons", () => {
  const evs = releaseEvents(["ShiftLeft", "KeyA"], [0, 2]);
  // every event is protocol-valid
  evs.forEach((e) => expect(() => parseInputEvent(e)).not.toThrow());
  expect(evs).toContainEqual({ t: "kup", code: "ShiftLeft" });
  expect(evs).toContainEqual({ t: "kup", code: "KeyA" });
  expect(evs).toContainEqual({ t: "mup", button: "left" });
  expect(evs).toContainEqual({ t: "mup", button: "right" });
  expect(evs.length).toBe(4);
});

test("releaseEvents skips unknown button ids", () => {
  expect(releaseEvents([], [5])).toEqual([]);
});
