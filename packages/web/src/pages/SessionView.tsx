import { useEffect, useRef, useState } from "react";
import type { Device } from "@rd/protocol";
import { API_BASE } from "../api.js";
import { connectSession, type ConnectionState, type Session } from "../rtc.js";

export interface SessionViewProps {
  token: string;
  device: Device;
  onExit: () => void;
}

interface EchoEntry {
  sent: string;
  reply?: string;
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

/**
 * Remote session view. Opens a WebRTC "echo" data channel to the selected
 * device via the signaling server, lets the operator send text, and shows the
 * echoed replies alongside the live connection state.
 */
export function SessionView({ token, device, onExit }: SessionViewProps) {
  const [state, setState] = useState<ConnectionState>("connecting");
  const [error, setError] = useState<string | null>(null);
  const [entries, setEntries] = useState<EchoEntry[]>([]);
  const [draft, setDraft] = useState("");
  const sessionRef = useRef<Session | null>(null);
  // Tracks which sent message is awaiting its echo (FIFO).
  const pendingRef = useRef<number>(0);

  useEffect(() => {
    setState("connecting");
    setError(null);
    setEntries([]);
    pendingRef.current = 0;

    const session = connectSession(API_BASE, token, device.id, {
      onState: (s) => setState(s),
      onError: (message) => setError(message),
      onEcho: (text) => {
        setEntries((prev) => {
          const next = [...prev];
          // Attach the echo to the oldest message still awaiting a reply.
          const idx = next.findIndex((e) => e.reply === undefined);
          if (idx >= 0) next[idx] = { ...next[idx], reply: text };
          else next.push({ sent: "", reply: text });
          return next;
        });
      },
    });
    sessionRef.current = session;

    return () => {
      session.close();
      sessionRef.current = null;
    };
  }, [token, device.id]);

  function send() {
    const text = draft.trim();
    if (!text || state !== "connected") return;
    sessionRef.current?.send(text);
    setEntries((prev) => [...prev, { sent: text }]);
    setDraft("");
  }

  const connected = state === "connected";

  return (
    <div style={{ maxWidth: 560, margin: "5vh auto", fontFamily: "system-ui" }}>
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
        Device <code>{device.id}</code> — data-channel echo
      </p>

      {error && <p style={{ color: "crimson" }} role="alert">{error}</p>}

      <div
        style={{
          border: "1px solid #eee", borderRadius: 8, padding: 12, minHeight: 160,
          marginBottom: 12, background: "#fafafa",
        }}
      >
        {entries.length === 0 && (
          <p style={{ color: "#aaa", margin: 0 }}>No messages yet. Send one below.</p>
        )}
        {entries.map((e, i) => (
          <div key={i} style={{ marginBottom: 8 }}>
            {e.sent && (
              <div style={{ textAlign: "right" }}>
                <span style={{ background: "#dbeafe", padding: "4px 8px", borderRadius: 6 }}>
                  {e.sent}
                </span>
              </div>
            )}
            {e.reply !== undefined && (
              <div style={{ textAlign: "left", marginTop: 2 }}>
                <span
                  data-testid="echo-reply"
                  style={{ background: "#dcfce7", padding: "4px 8px", borderRadius: 6 }}
                >
                  {e.reply}
                </span>
              </div>
            )}
          </div>
        ))}
      </div>

      <form
        onSubmit={(ev) => {
          ev.preventDefault();
          send();
        }}
        style={{ display: "flex", gap: 8 }}
      >
        <input
          style={{ flex: 1 }}
          placeholder={connected ? "Type a message to echo" : "Waiting for connection…"}
          value={draft}
          disabled={!connected}
          onChange={(ev) => setDraft(ev.target.value)}
        />
        <button type="submit" disabled={!connected || draft.trim().length === 0}>
          Send
        </button>
      </form>
    </div>
  );
}
