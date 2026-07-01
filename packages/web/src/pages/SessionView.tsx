import type { Device } from "@rd/protocol";

export interface SessionViewProps {
  token: string;
  device: Device;
  onExit: () => void;
}

/**
 * Placeholder for the remote session view. The full WebRTC signaling +
 * media/input handling lands in Task 4; this just confirms a device was
 * selected and offers a way back to the device list.
 */
export function SessionView({ device, onExit }: SessionViewProps) {
  return (
    <div style={{ padding: 24, fontFamily: "system-ui" }}>
      <button onClick={onExit}>← Back to devices</button>
      <h2>Session: {device.name}</h2>
      <p>Connecting to <code>{device.id}</code>…</p>
      <p style={{ color: "#888" }}>Remote session (WebRTC) coming soon.</p>
    </div>
  );
}
