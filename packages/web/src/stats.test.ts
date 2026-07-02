import { describe, it, expect } from "vitest";
import { parseVideoStats } from "./stats.js";

function fakeReport(entries: Record<string, unknown>[]): RTCStatsReport {
  const m = new Map<string, unknown>();
  entries.forEach((e, i) => m.set(String(i), e));
  return m as unknown as RTCStatsReport;
}

describe("parseVideoStats", () => {
  it("reads fps, resolution, and rtt from a single sample", () => {
    const report = fakeReport([
      { type: "inbound-rtp", kind: "video", framesPerSecond: 30, frameWidth: 1280, frameHeight: 720, bytesReceived: 1000, timestamp: 1000 },
      { type: "candidate-pair", state: "succeeded", nominated: true, currentRoundTripTime: 0.042 },
    ]);
    const { stats, sample } = parseVideoStats(report, null);
    expect(stats.fps).toBe(30);
    expect(stats.width).toBe(1280);
    expect(stats.height).toBe(720);
    expect(stats.rttMs).toBe(42);
    expect(stats.kbps).toBe(0); // no prev sample → no bitrate yet
    expect(sample).toEqual({ bytesReceived: 1000, timestamp: 1000 });
  });

  it("computes kbps from the delta vs the previous sample", () => {
    const report = fakeReport([
      { type: "inbound-rtp", kind: "video", framesPerSecond: 30, frameWidth: 1280, frameHeight: 720, bytesReceived: 126000, timestamp: 2000 },
    ]);
    // 125000 bytes over 1.0s = 1,000,000 bits/s = 1000 kbps
    const { stats } = parseVideoStats(report, { bytesReceived: 1000, timestamp: 1000 });
    expect(stats.kbps).toBe(1000);
  });

  it("returns null rtt when there is no nominated pair", () => {
    const report = fakeReport([
      { type: "inbound-rtp", kind: "video", framesPerSecond: 24, bytesReceived: 0, timestamp: 0 },
    ]);
    expect(parseVideoStats(report, null).stats.rttMs).toBeNull();
  });
});
