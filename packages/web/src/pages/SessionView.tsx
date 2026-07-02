import { useCallback, useEffect, useRef, useState } from "react";
import type { Device, InputEvent } from "@rd/protocol";
import { API_BASE } from "../api.js";
import {
  connectSession,
  contentRect,
  mouseButtonName,
  mouseCoords,
  releaseEvents,
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
 * Remote session view. A `<video>` element renders the remote screen's
 * MediaStream track and doubles as the focusable capture surface for
 * mouse/keyboard: each input is sent as an InputEvent over the data channel,
 * and the most recent events are logged so the operator can see input is
 * transmitting. Injection happens on the agent (被控端).
 */
export function SessionView({ token, device, onExit }: SessionViewProps) {
  const [state, setState] = useState<ConnectionState>("connecting");
  const [error, setError] = useState<string | null>(null);
  const [log, setLog] = useState<string[]>([]);
  const [isFullscreen, setIsFullscreen] = useState(false);
  const sessionRef = useRef<Session | null>(null);
  const surfaceRef = useRef<HTMLVideoElement | null>(null);
  const videoRef = useRef<HTMLVideoElement | null>(null);
  // rAF coalescing for mousemove: keep only the latest position per frame.
  const pendingMove = useRef<{ x: number; y: number } | null>(null);
  const rafId = useRef<number | null>(null);
  // Currently-held keys/buttons, so we can release them all if the capture
  // surface loses focus (nothing should stick down remotely).
  const pressedKeys = useRef<Set<string>>(new Set());
  const pressedButtons = useRef<Set<number>>(new Set());

  useEffect(() => {
    setState("connecting");
    setError(null);
    setLog([]);
    const session = connectSession(API_BASE, token, device.id, {
      onState: setState,
      onError: setError,
      onRemoteStream: (stream) => {
        if (videoRef.current) videoRef.current.srcObject = stream;
      },
    });
    sessionRef.current = session;
    return () => {
      releaseAll();
      session.close();
      sessionRef.current = null;
      if (rafId.current !== null) cancelAnimationFrame(rafId.current);
    };
  }, [token, device.id]);

  const connected = state === "connected";
  const connectedRef = useRef(false);
  connectedRef.current = connected;

  const emit = useCallback((ev: InputEvent) => {
    sessionRef.current?.sendInput(ev);
    setLog((prev) => [describe(ev), ...prev].slice(0, 20));
  }, []);

  // Release every currently-held key/button (e.g. on blur/mouse-leave/unmount)
  // so nothing sticks down on the remote side once capture is interrupted.
  function releaseAll() {
    for (const ev of releaseEvents([...pressedKeys.current], [...pressedButtons.current])) {
      sessionRef.current?.sendInput(ev);
    }
    pressedKeys.current.clear();
    pressedButtons.current.clear();
  }

  useEffect(() => {
    const el = surfaceRef.current;
    if (!el) return;
    const onWheelNative = (e: WheelEvent) => {
      if (!connectedRef.current) return;
      e.preventDefault();
      emit({ t: "wheel", dx: e.deltaX, dy: e.deltaY });
    };
    el.addEventListener("wheel", onWheelNative, { passive: false });
    return () => el.removeEventListener("wheel", onWheelNative);
  }, [emit]);

  // Fullscreen: fill the whole monitor with the remote screen. Track the state
  // (the user can also exit with Esc / the OS), focusing the surface on enter so
  // keyboard capture works immediately, and releasing held keys on exit.
  useEffect(() => {
    const onFsChange = () => {
      const fs = document.fullscreenElement === surfaceRef.current;
      setIsFullscreen(fs);
      if (fs) surfaceRef.current?.focus();
      else releaseAll();
    };
    document.addEventListener("fullscreenchange", onFsChange);
    return () => document.removeEventListener("fullscreenchange", onFsChange);
  }, []);

  const toggleFullscreen = useCallback(() => {
    const el = surfaceRef.current;
    if (document.fullscreenElement) {
      void document.exitFullscreen();
    } else {
      void el?.requestFullscreen?.();
    }
  }, []);

  function onMouseMove(e: React.MouseEvent) {
    if (!connected || !surfaceRef.current) return;
    const el = surfaceRef.current;
    const rect = el.getBoundingClientRect();
    const box = contentRect({ width: rect.width, height: rect.height }, el.videoWidth, el.videoHeight);
    const adj = { left: rect.left + box.left, top: rect.top + box.top, width: box.width, height: box.height };
    pendingMove.current = mouseCoords(e.clientX, e.clientY, adj);
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
    if (button) {
      pressedButtons.current.add(e.button);
      emit({ t: "mdown", button });
    }
  }

  function onMouseUp(e: React.MouseEvent) {
    if (!connected) return;
    const button = mouseButtonName(e.button);
    pressedButtons.current.delete(e.button);
    if (button) emit({ t: "mup", button });
  }

  function onKeyDown(e: React.KeyboardEvent) {
    if (!connected) return;
    if (e.code === "Escape") {
      surfaceRef.current?.blur();
      return;
    }
    e.preventDefault();
    pressedKeys.current.add(e.code);
    emit({ t: "kdown", code: e.code });
  }

  function onKeyUp(e: React.KeyboardEvent) {
    if (!connected) return;
    e.preventDefault();
    pressedKeys.current.delete(e.code);
    emit({ t: "kup", code: e.code });
  }

  return (
    <div style={{ maxWidth: 720, margin: "5vh auto", fontFamily: "system-ui" }}>
      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
        <button onClick={onExit}>← Back to devices</button>
        <span style={{ display: "flex", alignItems: "center", gap: 12 }}>
          <button onClick={toggleFullscreen} disabled={!connected} data-testid="fullscreen-btn">
            {isFullscreen ? "Exit fullscreen" : "⛶ Fullscreen"}
          </button>
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

      <video
        ref={(el) => {
          surfaceRef.current = el;
          videoRef.current = el;
        }}
        data-testid="remote-surface"
        tabIndex={0}
        autoPlay
        muted
        playsInline
        onMouseMove={onMouseMove}
        onMouseDown={onMouseDown}
        onMouseUp={onMouseUp}
        onMouseLeave={releaseAll}
        onKeyDown={onKeyDown}
        onKeyUp={onKeyUp}
        onBlur={releaseAll}
        onContextMenu={(e) => e.preventDefault()}
        style={
          isFullscreen
            ? {
                // Fullscreen element: fill the monitor, letterbox the remote
                // screen (aspect-fit), no chrome.
                width: "100%", height: "100%", background: "#000",
                objectFit: "contain", outline: "none", border: "none",
                cursor: connected ? "crosshair" : "default",
              }
            : {
                width: "100%", height: 360, borderRadius: 8, border: "2px solid #cbd5e1",
                background: "#0f172a", outline: "none", objectFit: "contain",
                cursor: connected ? "crosshair" : "default",
              }
        }
      />

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
