import type { WebSocketServer, WebSocket } from "ws";
import { parseSignalingMessage } from "@rd/protocol";
import type { DevicesRepo } from "../repo/devices.js";
import { verifyToken } from "../auth.js";
import type { Config } from "../config.js";
import { Registry, type Conn } from "./registry.js";

export interface HubDeps { devices: DevicesRepo; config: Config; registry: Registry; }

function wrap(ws: WebSocket): Conn {
  return { send: (d) => ws.send(d), close: () => ws.close() };
}

export function attachSignaling(wss: WebSocketServer, deps: HubDeps) {
  const { devices, config, registry } = deps;
  wss.on("connection", (ws, req) => {
    const conn = wrap(ws);
    // Web 端用 ?token=<jwt> 鉴权；Agent 端发 agent-online 携带 device token
    const url = new URL(req.url ?? "/", "http://localhost");
    const jwt = url.searchParams.get("token");
    let webUserId: string | undefined;
    if (jwt) { try { webUserId = verifyToken(jwt, config.jwtSecret).userId; } catch { ws.close(); return; } }

    ws.on("message", (raw) => {
      let msg;
      try { msg = parseSignalingMessage(JSON.parse(raw.toString())); }
      catch { conn.send(JSON.stringify({ type: "error", code: "bad-message", message: "unparseable" })); return; }

      switch (msg.type) {
        case "agent-online": {
          const dev = devices.findByToken(msg.token);
          if (!dev) { conn.send(JSON.stringify({ type: "error", code: "bad-token", message: "invalid device token" })); ws.close(); return; }
          registry.setAgentOnline(dev.id, conn);
          break;
        }
        case "connect": {
          // 只有已认证的 web 客户端可发起连接
          if (!webUserId) { conn.send(JSON.stringify({ type: "error", code: "unauthorized", message: "authentication required" })); return; }
          // 校验目标设备存在且归当前用户所有（未拥有/不存在返回同一错误，避免设备枚举）
          const device = devices.findById(msg.deviceId);
          if (!device || device.userId !== webUserId) { conn.send(JSON.stringify({ type: "error", code: "forbidden", message: "device not found or not owned" })); return; }
          const agent = registry.getAgent(msg.deviceId);
          if (!agent) { conn.send(JSON.stringify({ type: "error", code: "offline", message: "device offline" })); return; }
          const sessionId = registry.createSession(conn, agent);
          const ice = { relayPolicy: config.relayPolicy, iceServers: config.iceServers };
          agent.send(JSON.stringify({ type: "incoming", sessionId, ...ice }));
          conn.send(JSON.stringify({ type: "session-ready", sessionId, ...ice }));
          break;
        }
        case "sdp": case "ice": {
          const peer = registry.peerOf(conn, msg.sessionId);
          if (peer) peer.send(JSON.stringify(msg));
          break;
        }
        case "peer-left": {
          const peer = registry.peerOf(conn, msg.sessionId);
          if (peer) peer.send(JSON.stringify(msg));
          registry.dropSession(msg.sessionId);
          break;
        }
      }
    });
    ws.on("close", () => {
      for (const { sessionId, peer } of registry.remove(conn)) {
        try {
          peer.send(JSON.stringify({ type: "peer-left", sessionId }));
        } catch {
          /* peer already gone; ignore */
        }
      }
    });
  });
}
