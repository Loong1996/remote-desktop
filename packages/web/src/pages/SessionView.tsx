import { useEffect, useRef, useState } from "react";
import type { Device, InputEvent } from "@rd/protocol";
import { API_BASE } from "../api.js";
import {
  connectSession,
  mouseButtonName,
  mouseCoords,
  type ConnectionState,
  type Session,
} from "../rtc.js";

export interface SessionViewProps {
  token: string;
  device: Device;
  onExit: () => void;
}

const STATE_LABEL: Record<ConnectionState, string> = {
  connecting: "Connecting…",
  signaling: "Negotiating…",
  connected: "Connected",
  closed: "Disconnected",
  error: "Error",
};

const STATE_COLOR: Record<ConnectionState, string> = {
  connecting: "#f59e0b",
  signaling: "#f59e0b",
  connected: "#22c55e",
  closed: "#9ca3af",
  error: "crimson",
};

function describe(ev: InputEvent): string {
  switch (ev.t) {
    case "mmove":
      return `mmove ${ev.x.toFixed(2)},${ev.y.toFixed(2)}`;
    case "mdown":
      return `mdown ${ev.button}`;
    case "mup":
      return `mup ${ev.button}`;
    case "wheel":
      return `wheel ${ev.dx.toFixed(0)},${ev.dy.toFixed(0)}`;
    case "kdown":
      return `kdown ${ev.code}`;
    case "kup":
      return `kup ${ev.code}`;
  }
}

/**
 * Remote session view. Until video lands (Plan 4), a focusable placeholder
 * "remote screen" captures mouse/keyboard, sends each as an InputEvent over the
 * data channel, and logs the most recent events so the operator can see input
 * is transmitting. Injection happens on the agent (被控端).
 */
export function SessionView({ token, device, onExit }: SessionViewProps) {
  const [state, setState] = useState<ConnectionState>("connecting");
  const [error, setError] = useState<string | null>(null);
  const [log, setLog] = useState<string[]>([]);
  const sessionRef = useRef<Session | null>(null);
  const surfaceRef = useRef<HTMLDivElement | null>(null);
  // rAF coalescing for mousemove: keep only the latest position per frame.
  const pendingMove = useRef<{ x: number; y: number } | null>(null);
  const rafId = useRef<number | null>(null);

  useEffect(() => {
    setState("connecting");
    setError(null);
    setLog([]);
    const session = connectSession(API_BASE, token, device.id, {
      onState: setState,
      onError: setError,
    });
    sessionRef.current = session;
    return () => {
      session.close();
      sessionRef.current = null;
      if (rafId.current !== null) cancelAnimationFrame(rafId.current);
    };
  }, [token, device.id]);

  const connected = state === "connected";

  function emit(ev: InputEvent) {
    sessionRef.current?.sendInput(ev);
    setLog((prev) => [describe(ev), ...prev].slice(0, 20));
  }

  function onMouseMove(e: React.MouseEvent) {
    if (!connected || !surfaceRef.current) return;
    const rect = surfaceRef.current.getBoundingClientRect();
    pendingMove.current = mouseCoords(e.clientX, e.clientY, rect);
    if (rafId.current === null) {
      rafId.current = requestAnimationFrame(() => {
        rafId.current = null;
        const p = pendingMove.current;
        pendingMove.current = null;
        if (p) emit({ t: "mmove", x: p.x, y: p.y });
      });
    }
  }

  function onMouseDown(e: React.MouseEvent) {
    if (!connected) return;
    const button = mouseButtonName(e.button);
    if (button) emit({ t: "mdown", button });
  }

  function onMouseUp(e: React.MouseEvent) {
    if (!connected) return;
    const button = mouseButtonName(e.button);
    if (button) emit({ t: "mup", button });
  }

  function onWheel(e: React.WheelEvent) {
    if (!connected) return;
    emit({ t: "wheel", dx: e.deltaX, dy: e.deltaY });
  }

  function onKeyDown(e: React.KeyboardEvent) {
    if (!connected) return;
    if (e.code === "Escape") {
      surfaceRef.current?.blur();
      return;
    }
    e.preventDefault();
    emit({ t: "kdown", code: e.code });
  }

  function onKeyUp(e: React.KeyboardEvent) {
    if (!connected) return;
    e.preventDefault();
    emit({ t: "kup", code: e.code });
  }

  return (
    <div style={{ maxWidth: 720, margin: "5vh auto", fontFamily: "system-ui" }}>
      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
        <button onClick={onExit}>← Back to devices</button>
        <span style={{ display: "flex", alignItems: "center", gap: 8 }}>
          <span
            aria-label={STATE_LABEL[state]}
            title={STATE_LABEL[state]}
            style={{
              display: "inline-block", width: 10, height: 10, borderRadius: "50%",
              background: STATE_COLOR[state],
            }}
          />
          <span data-testid="conn-state">{STATE_LABEL[state]}</span>
        </span>
      </div>

      <h2>Session: {device.name}</h2>
      <p style={{ color: "#888" }}>
        Click the panel to capture — mouse & keyboard are sent to <code>{device.id}</code>. Press Esc to release.
      </p>

      {error && <p style={{ color: "crimson" }} role="alert">{error}</p>}

      <div
        ref={surfaceRef}
        data-testid="remote-surface"
        tabIndex={0}
        onMouseMove={onMouseMove}
        onMouseDown={onMouseDown}
        onMouseUp={onMouseUp}
        onWheel={onWheel}
        onKeyDown={onKeyDown}
        onKeyUp={onKeyUp}
        onContextMenu={(e) => e.preventDefault()}
        style={{
          height: 360, borderRadius: 8, border: "2px dashed #cbd5e1",
          background: connected ? "#0f172a" : "#f1f5f9",
          color: connected ? "#94a3b8" : "#94a3b8",
          display: "flex", alignItems: "center", justifyContent: "center",
          textAlign: "center", outline: "none", userSelect: "none", cursor: connected ? "crosshair" : "default",
        }}
      >
        {connected ? "Remote screen (no video yet — input captured here)" : "Waiting for connection…"}
      </div>

      <h3 style={{ marginBottom: 4 }}>Sent events</h3>
      <div
        style={{
          border: "1px solid #eee", borderRadius: 8, padding: 8, height: 140,
          overflowY: "auto", background: "#fafafa", fontFamily: "ui-monospace, monospace", fontSize: 12,
        }}
      >
        {log.length === 0 && <p style={{ color: "#aaa", margin: 0 }}>No events yet.</p>}
        {log.map((line, i) => (
          <div key={i} data-testid="event-line">{line}</div>
        ))}
      </div>
    </div>
  );
}
