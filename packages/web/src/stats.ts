export interface VideoStats {
  fps: number;
  kbps: number;
  rttMs: number | null;
  width: number;
  height: number;
}

/** The raw fields needed to compute a bitrate delta between two polls. */
export interface StatsSample {
  bytesReceived: number;
  timestamp: number; // ms
}

/** Extract a display-ready VideoStats from a getStats() report. `prev` is the
 *  sample from the previous poll, used for the byte/time delta → bitrate. */
export function parseVideoStats(
  report: RTCStatsReport,
  prev: StatsSample | null,
): { stats: VideoStats; sample: StatsSample } {
  let inbound: Record<string, unknown> | null = null;
  let pair: Record<string, unknown> | null = null;
  report.forEach((s: unknown) => {
    const r = s as Record<string, unknown>;
    if (r.type === "inbound-rtp" && r.kind === "video") inbound = r;
    if (r.type === "candidate-pair" && r.state === "succeeded" && r.nominated === true) pair = r;
  });

  const num = (o: Record<string, unknown> | null, k: string): number =>
    typeof o?.[k] === "number" ? (o[k] as number) : 0;

  const bytesReceived = num(inbound, "bytesReceived");
  const timestamp = num(inbound, "timestamp");
  const sample: StatsSample = { bytesReceived, timestamp };

  let kbps = 0;
  if (prev && timestamp > prev.timestamp) {
    const bits = (bytesReceived - prev.bytesReceived) * 8;
    const seconds = (timestamp - prev.timestamp) / 1000;
    if (seconds > 0) kbps = Math.max(0, Math.round(bits / seconds / 1000));
  }

  const rttRaw = pair && typeof (pair as Record<string, unknown>).currentRoundTripTime === "number"
    ? ((pair as Record<string, unknown>).currentRoundTripTime as number)
    : null;

  return {
    stats: {
      fps: Math.round(num(inbound, "framesPerSecond")),
      kbps,
      rttMs: rttRaw === null ? null : Math.round(rttRaw * 1000),
      width: num(inbound, "frameWidth"),
      height: num(inbound, "frameHeight"),
    },
    sample,
  };
}
