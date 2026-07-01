import {
  parseSignalingMessage,
  type Connect,
  type Sdp,
  type Ice,
  type IceServer,
  type InputEvent,
  type MouseButton,
} from "@rd/protocol";

// ---------------------------------------------------------------------------
// Pure, testable helpers (no browser APIs). jsdom has no real WebRTC/WebSocket,
// so the message-construction + url-derivation logic is factored out here and
// unit-tested; the live handshake is exercised by the manual e2e in Task 5.
// ---------------------------------------------------------------------------

/**
 * Derive the signaling WebSocket URL from the HTTP server base URL, carrying
 * the JWT as a `?token=` query parameter. `http` → `ws`, `https` → `wss`;
 * any explicit path/port on the base URL is preserved.
 */
export function deriveWsUrl(serverUrl: string, token: string): string {
  const u = new URL(serverUrl);
  u.protocol = u.protocol === "https:" ? "wss:" : "ws:";
  u.searchParams.set("token", token);
  return u.toString();
}

/** `{type:"connect",deviceId}` — sent by the web control end to open a session. */
export function buildConnect(deviceId: string): Connect {
  return { type: "connect", deviceId };
}

/** Wrap a local SDP offer into the `sdp` signaling message. */
export function buildOffer(sessionId: string, offer: RTCSessionDescriptionInit): Sdp {
  return {
    type: "sdp",
    sessionId,
    // offer.type is always "offer" here; assert for the protocol's narrow union.
    sdp: { type: "offer", sdp: offer.sdp ?? "" },
  };
}

/** Wrap a local ICE candidate (already `.toJSON()`'d) into the `ice` message. */
export function buildIce(sessionId: string, candidate: unknown): Ice {
  return { type: "ice", sessionId, candidate };
}

/** Map an absolute pointer position to relative [0,1] coords within `rect`. */
export function mouseCoords(
  clientX: number,
  clientY: number,
  rect: { left: number; top: number; width: number; height: number },
): { x: number; y: number } {
  const clamp = (n: number) => Math.min(1, Math.max(0, n));
  return {
    x: clamp((clientX - rect.left) / rect.width),
    y: clamp((clientY - rect.top) / rect.height),
  };
}

/** The `object-fit: contain` content box (offsets relative to the element's
 *  top-left) for a video of intrinsic size `videoW`×`videoH` shown in an
 *  element of size `el`. Falls back to the full element when the size is unknown. */
export function contentRect(
  el: { width: number; height: number },
  videoW: number,
  videoH: number,
): { left: number; top: number; width: number; height: number } {
  if (videoW <= 0 || videoH <= 0) {
    return { left: 0, top: 0, width: el.width, height: el.height };
  }
  const scale = Math.min(el.width / videoW, el.height / videoH);
  const width = videoW * scale;
  const height = videoH * scale;
  return { left: (el.width - width) / 2, top: (el.height - height) / 2, width, height };
}

/** Map a DOM `MouseEvent.button` id to the protocol button name (or null). */
export function mouseButtonName(button: number): MouseButton | null {
  switch (button) {
    case 0:
      return "left";
    case 1:
      return "middle";
    case 2:
      return "right";
    default:
      return null;
  }
}

/** Resolve the MediaStream for an incoming track: prefer the negotiated
 *  stream, else build one from the track. `mk` is injectable for testing. */
export function streamFromTrackEvent(
  ev: RTCTrackEvent,
  mk: (t: MediaStreamTrack) => MediaStream = (t) => new MediaStream([t]),
): MediaStream {
  return ev.streams[0] ?? mk(ev.track);
}

// ---------------------------------------------------------------------------
// Live session orchestration (browser WebRTC + WebSocket).
// ---------------------------------------------------------------------------

export type ConnectionState =
  | "connecting" // WS opening / waiting for session-ready
  | "signaling" // exchanging SDP/ICE
  | "connected" // data channel open
  | "closed"
  | "error";

export interface SessionCallbacks {
  /** Called on every connection-state transition. */
  onState?: (state: ConnectionState) => void;
  /** Called on any fatal error (WS error, signaling error message, etc.). */
  onError?: (message: string) => void;
  /** Called when the remote peer's video track arrives. */
  onRemoteStream?: (stream: MediaStream) => void;
}

export interface Session {
  /** Send an InputEvent over the "input" data channel (no-op until open). */
  sendInput: (ev: InputEvent) => void;
  /** Tear down the data channel, peer connection, and WebSocket. */
  close: () => void;
}

/**
 * Connect to a device via the signaling server and open an "input" data channel.
 *
 * Flow (web is the offerer):
 *   WS open → send {connect,deviceId}
 *   ← session-ready{sessionId,iceServers} → new RTCPeerConnection, createDataChannel("input"),
 *     createOffer → setLocalDescription → send {sdp,offer}
 *   pc.onicecandidate → send {ice}
 *   ← sdp{answer} → setRemoteDescription
 *   ← ice → addIceCandidate
 *   channel.onopen → connected
 */
export function connectSession(
  serverUrl: string,
  token: string,
  deviceId: string,
  callbacks: SessionCallbacks = {},
): Session {
  const { onState, onError, onRemoteStream } = callbacks;

  let pc: RTCPeerConnection | null = null;
  let channel: RTCDataChannel | null = null;
  let sessionId: string | null = null;
  let closed = false;

  const ws = new WebSocket(deriveWsUrl(serverUrl, token));

  function setState(state: ConnectionState): void {
    onState?.(state);
  }

  function fail(message: string): void {
    if (closed) return;
    onError?.(message);
    setState("error");
    close();
  }

  function sendWs(msg: unknown): void {
    if (ws.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify(msg));
    }
  }

  function close(): void {
    if (closed) return;
    closed = true;
    try {
      channel?.close();
    } catch {
      /* ignore */
    }
    try {
      pc?.close();
    } catch {
      /* ignore */
    }
    try {
      if (ws.readyState === WebSocket.OPEN || ws.readyState === WebSocket.CONNECTING) {
        ws.close();
      }
    } catch {
      /* ignore */
    }
    channel = null;
    pc = null;
    setState("closed");
  }

  setState("connecting");

  ws.onopen = () => {
    sendWs(buildConnect(deviceId));
  };

  ws.onerror = () => {
    fail("signaling connection error");
  };

  ws.onclose = () => {
    if (!closed) {
      closed = true;
      channel = null;
      pc = null;
      setState("closed");
    }
  };

  function startPeer(iceServers: IceServer[], sid: string): void {
    sessionId = sid;
    setState("signaling");

    pc = new RTCPeerConnection({ iceServers: iceServers as RTCIceServer[] });

    pc.onicecandidate = (e) => {
      if (e.candidate && sessionId) {
        sendWs(buildIce(sessionId, e.candidate.toJSON()));
      }
    };

    pc.onconnectionstatechange = () => {
      if (!pc) return;
      if (pc.connectionState === "failed") {
        fail("peer connection failed");
      }
    };

    pc.addTransceiver("video", { direction: "recvonly" });
    pc.ontrack = (ev) => {
      onRemoteStream?.(streamFromTrackEvent(ev));
    };

    channel = pc.createDataChannel("input");
    channel.onopen = () => {
      setState("connected");
    };
    channel.onclose = () => {
      if (!closed) setState("closed");
    };

    void (async () => {
      try {
        const offer = await pc!.createOffer();
        await pc!.setLocalDescription(offer);
        sendWs(buildOffer(sid, offer));
      } catch (err) {
        fail(err instanceof Error ? err.message : "failed to create offer");
      }
    })();
  }

  ws.onmessage = (e) => {
    let raw: unknown;
    try {
      raw = JSON.parse(typeof e.data === "string" ? e.data : String(e.data));
    } catch {
      return; // ignore non-JSON frames
    }

    // session-ready / error are server→client only and not covered by
    // parseSignalingMessage, so handle them by hand before delegating.
    if (typeof raw === "object" && raw !== null) {
      const rec = raw as Record<string, unknown>;
      if (rec.type === "session-ready") {
        const sid = rec.sessionId;
        const iceServers = (rec.iceServers as IceServer[]) ?? [];
        if (typeof sid === "string") startPeer(iceServers, sid);
        return;
      }
      if (rec.type === "error") {
        const message =
          typeof rec.message === "string" ? rec.message : "signaling error";
        fail(message);
        return;
      }
      if (rec.type === "peer-left") {
        close();
        return;
      }
    }

    let msg;
    try {
      msg = parseSignalingMessage(raw);
    } catch {
      return; // ignore unrecognized frames
    }

    if (msg.type === "sdp" && msg.sdp.type === "answer") {
      void pc
        ?.setRemoteDescription(msg.sdp as RTCSessionDescriptionInit)
        .catch((err: unknown) =>
          fail(err instanceof Error ? err.message : "failed to set remote answer"),
        );
    } else if (msg.type === "ice") {
      void pc
        ?.addIceCandidate(msg.candidate as RTCIceCandidateInit)
        .catch(() => {
          /* candidate may arrive before remote desc / be non-fatal; ignore */
        });
    }
  };

  return {
    sendInput(ev: InputEvent) {
      if (channel && channel.readyState === "open") {
        channel.send(JSON.stringify(ev));
      }
    },
    close,
  };
}
