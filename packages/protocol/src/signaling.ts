export type RelayPolicy = "direct-only" | "relay-fallback" | "force-relay";

export interface IceServer { urls: string | string[]; username?: string; credential?: string; }

/** Agent 上线：用 device token 认证 */
export interface AgentOnline { type: "agent-online"; token: string; }
/** Web 发起连接某设备 */
export interface Connect { type: "connect"; deviceId: string; }
/** 服务端通知 Agent 有入站会话 */
export interface Incoming { type: "incoming"; sessionId: string; relayPolicy: RelayPolicy; iceServers: IceServer[]; }
/** 服务端告知 Web 会话已建立、附 ICE 配置 */
export interface SessionReady { type: "session-ready"; sessionId: string; relayPolicy: RelayPolicy; iceServers: IceServer[]; }
/** SDP 转发（offer/answer） */
export interface Sdp { type: "sdp"; sessionId: string; sdp: { type: "offer" | "answer"; sdp: string }; }
/** ICE candidate 转发 */
export interface Ice { type: "ice"; sessionId: string; candidate: unknown; }
/** 对端离开 */
export interface PeerLeft { type: "peer-left"; sessionId: string; }
/** 错误 */
export interface ErrorMsg { type: "error"; code: string; message: string; }

export type SignalingMessage =
  | AgentOnline | Connect | Incoming | SessionReady | Sdp | Ice | PeerLeft | ErrorMsg;

function isObj(v: unknown): v is Record<string, unknown> {
  return typeof v === "object" && v !== null;
}
function str(v: unknown, field: string): string {
  if (typeof v !== "string" || v.length === 0) throw new Error(`invalid field: ${field}`);
  return v;
}

export function parseSignalingMessage(raw: unknown): SignalingMessage {
  if (!isObj(raw)) throw new Error("message must be an object");
  const t = raw.type;
  switch (t) {
    case "agent-online": return { type: t, token: str(raw.token, "token") };
    case "connect": return { type: t, deviceId: str(raw.deviceId, "deviceId") };
    case "sdp": {
      const sdp = raw.sdp;
      if (!isObj(sdp) || (sdp.type !== "offer" && sdp.type !== "answer")) throw new Error("invalid sdp");
      return { type: t, sessionId: str(raw.sessionId, "sessionId"), sdp: { type: sdp.type, sdp: str(sdp.sdp, "sdp.sdp") } };
    }
    case "ice": return { type: t, sessionId: str(raw.sessionId, "sessionId"), candidate: raw.candidate };
    case "peer-left": return { type: t, sessionId: str(raw.sessionId, "sessionId") };
    default: throw new Error(`unknown signaling type: ${String(t)}`);
  }
}
