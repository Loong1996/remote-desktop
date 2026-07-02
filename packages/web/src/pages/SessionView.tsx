import { useCallback, useEffect, useRef, useState } from "react";
import type { Device, InputEvent } from "@rd/protocol";
import { API_BASE } from "../api.js";
import { clipboardToSend, type ClipMode } from "../clipboard.js";
import { COMBOS, comboEvents } from "../combos.js";
import {
  connectSession,
  contentRect,
  mouseButtonName,
  mouseCoords,
  releaseEvents,
  type ConnectionState,
  type Session,
} from "../rtc.js";
import { parseVideoStats, type StatsSample, type VideoStats } from "../stats.js";

export interface SessionViewProps {
  token: string;
  device: Device;
  onExit: () => void;
}

// Bitrate presets. Capture stays at ~720p (see agent target_capture_size), so
// beyond ~10 Mbps the gain is marginal; 原画 is the agent's clamp ceiling
// (QUALITY_MAX_BPS = 20 Mbps) — visually artifact-free 720p, not native-res.
const QUALITY_PRESETS = [
  { label: "流畅", bps: 1_500_000 },
  { label: "均衡", bps: 3_000_000 },
  { label: "高清", bps: 6_000_000 },
  { label: "超清", bps: 10_000_000 },
  { label: "原画", bps: 20_000_000 },
];

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
  // "Fill window" = maximize inside the browser viewport (CSS overlay), distinct
  // from OS fullscreen (Fullscreen API). Keeps the browser chrome/tabs visible.
  const [isMaximized, setIsMaximized] = useState(false);
  const [showStats, setShowStats] = useState(false);
  const [stats, setStats] = useState<VideoStats | null>(null);
  const [bitrate, setBitrate] = useState(3_000_000);
  const [clipMode, setClipMode] = useState<ClipMode>("off");
  const lastClip = useRef<string>("");
  const statsSample = useRef<StatsSample | null>(null);
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
      onClipboard: (text) => {
        // Received a clip-set (manual pull reply, or both-mode auto push): mirror it
        // locally and record it so our own poller won't echo it back.
        lastClip.current = text;
        void navigator.clipboard.writeText(text).catch(() => {});
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

  const sendCombo = useCallback(
    (codes: string[]) => {
      for (const ev of comboEvents(codes)) emit(ev);
    },
    [emit],
  );

  const onQuality = useCallback((bps: number) => {
    setBitrate(bps);
    sessionRef.current?.sendControl({ t: "quality", bitrateBps: bps });
  }, []);

  const onClipModeChange = useCallback((mode: ClipMode) => {
    setClipMode(mode);
    sessionRef.current?.sendControl({ t: "clip-mode", mode });
  }, []);

  const sendMyClipboard = useCallback(async () => {
    try {
      const text = await navigator.clipboard.readText();
      const toSend = clipboardToSend(text, lastClip.current);
      if (toSend !== null) {
        lastClip.current = toSend;
        sessionRef.current?.sendControl({ t: "clip-set", text: toSend });
      }
    } catch {
      /* clipboard read denied / no focus */
    }
  }, []);

  const pullRemoteClipboard = useCallback(() => {
    sessionRef.current?.sendControl({ t: "clip-request" });
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

  // Poll WebRTC stats ~1/s while the HUD is on and connected.
  useEffect(() => {
    if (!connected || !showStats) {
      setStats(null);
      statsSample.current = null;
      return;
    }
    const id = setInterval(() => {
      void sessionRef.current?.getStats().then((report) => {
        if (!report) return;
        const { stats: s, sample } = parseVideoStats(report, statsSample.current);
        statsSample.current = sample;
        setStats(s);
      });
    }, 1000);
    return () => clearInterval(id);
  }, [connected, showStats]);

  // Auto-sync local → remote in oneway/both while the tab has focus.
  useEffect(() => {
    if (!connected || clipMode === "off") return;
    const id = setInterval(() => {
      if (!document.hasFocus()) return;
      void navigator.clipboard
        .readText()
        .then((text) => {
          const toSend = clipboardToSend(text, lastClip.current);
          if (toSend !== null) {
            lastClip.current = toSend;
            sessionRef.current?.sendControl({ t: "clip-set", text: toSend });
          }
        })
        .catch(() => {});
    }, 800);
    return () => clearInterval(id);
  }, [connected, clipMode]);

  const toggleFullscreen = useCallback(() => {
    const el = surfaceRef.current;
    if (document.fullscreenElement) {
      void document.exitFullscreen();
    } else {
      void el?.requestFullscreen?.();
    }
  }, []);

  // Fill the browser viewport (CSS overlay), no Fullscreen API. Focus the
  // surface on enter so keyboard capture works immediately.
  const toggleMaximize = useCallback(() => {
    setIsMaximized((m) => {
      const next = !m;
      if (next) queueMicrotask(() => surfaceRef.current?.focus());
      return next;
    });
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
      if (isMaximized) setIsMaximized(false);
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
          <button onClick={toggleMaximize} disabled={!connected} data-testid="maximize-btn">
            {isMaximized ? "Exit fill" : "⤢ Fill window"}
          </button>
          <button onClick={toggleFullscreen} disabled={!connected} data-testid="fullscreen-btn">
            {isFullscreen ? "Exit fullscreen" : "⛶ Fullscreen"}
          </button>
          <button onClick={() => setShowStats((v) => !v)} disabled={!connected} data-testid="stats-btn">
            {showStats ? "Hide stats" : "📊 Stats"}
          </button>
          <select
            data-testid="quality-select"
            value={bitrate}
            disabled={!connected}
            onChange={(e) => onQuality(Number(e.target.value))}
            style={{ fontSize: 12 }}
          >
            {QUALITY_PRESETS.map((q) => (
              <option key={q.bps} value={q.bps}>{q.label}</option>
            ))}
          </select>
          <select
            data-testid="clip-mode"
            value={clipMode}
            disabled={!connected}
            onChange={(e) => onClipModeChange(e.target.value as ClipMode)}
            style={{ fontSize: 12 }}
          >
            <option value="off">剪贴板:手动</option>
            <option value="oneway">剪贴板:单向</option>
            <option value="both">剪贴板:双向</option>
          </select>
          <button
            data-testid="clip-send"
            disabled={!connected}
            onClick={() => void sendMyClipboard()}
            style={{ fontSize: 12 }}
          >
            发送剪贴板
          </button>
          <button
            data-testid="clip-pull"
            disabled={!connected}
            onClick={pullRemoteClipboard}
            style={{ fontSize: 12 }}
          >
            拉取远程
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

      <div
        data-testid="combo-bar"
        style={{ display: connected ? "flex" : "none", flexWrap: "wrap", gap: 6, margin: "8px 0" }}
      >
        {COMBOS.map((c) => (
          <button
            key={c.label}
            data-testid={`combo-${c.label}`}
            disabled={!connected}
            onClick={() => sendCombo(c.codes)}
            style={{ fontSize: 12 }}
          >
            {c.label}
          </button>
        ))}
      </div>

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
            : isMaximized
              ? {
                  // Fill the browser viewport (fixed overlay), browser chrome
                  // still visible; aspect-fit with letterbox.
                  position: "fixed", inset: 0, width: "100vw", height: "100vh",
                  zIndex: 1000, background: "#000", objectFit: "contain",
                  outline: "none", border: "none",
                  cursor: connected ? "crosshair" : "default",
                }
              : {
                  width: "100%", height: 360, borderRadius: 8, border: "2px solid #cbd5e1",
                  background: "#0f172a", outline: "none", objectFit: "contain",
                  cursor: connected ? "crosshair" : "default",
                }
        }
      />

      {showStats && (
        <div
          data-testid="stats-hud"
          style={{
            position: "fixed", top: 12, left: 12, zIndex: 1001,
            padding: "6px 10px", borderRadius: 6, background: "rgba(0,0,0,0.6)",
            color: "#fff", fontFamily: "ui-monospace, monospace", fontSize: 12,
          }}
        >
          {stats
            ? `${stats.fps} fps · ${stats.kbps} kbps · ${stats.rttMs ?? "?"} ms · ${stats.width}×${stats.height}`
            : "sampling…"}
        </div>
      )}

      {isMaximized && (
        <button
          onClick={toggleMaximize}
          data-testid="maximize-exit"
          style={{
            position: "fixed", top: 12, right: 12, zIndex: 1001,
            padding: "6px 10px", borderRadius: 6, border: "none",
            background: "rgba(0,0,0,0.6)", color: "#fff", cursor: "pointer",
          }}
        >
          Exit fill (Esc)
        </button>
      )}

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
