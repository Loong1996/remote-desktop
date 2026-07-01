import { expect, test, beforeEach } from "vitest";
import { Registry } from "../src/signaling/registry.js";

let reg: Registry;
beforeEach(() => { reg = new Registry(); });

test("agent online/offline tracked", () => {
  const conn = { send() {}, close() {} };
  reg.setAgentOnline("dev-1", conn as any);
  expect(reg.isOnline("dev-1")).toBe(true);
  reg.remove(conn as any);
  expect(reg.isOnline("dev-1")).toBe(false);
});

test("session links web and agent as peers", () => {
  const web = { send() {}, close() {} } as any;
  const agent = { send() {}, close() {} } as any;
  reg.setAgentOnline("dev-1", agent);
  const sid = reg.createSession(web, agent);
  expect(reg.peerOf(web, sid)).toBe(agent);
  expect(reg.peerOf(agent, sid)).toBe(web);
  reg.dropSession(sid);
  expect(reg.peerOf(web, sid)).toBeUndefined();
});

import { WebSocketServer, WebSocket } from "ws";
import { createServer } from "node:http";
import { openDb } from "../src/db.js";
import { UsersRepo } from "../src/repo/users.js";
import { DevicesRepo } from "../src/repo/devices.js";
import { attachSignaling } from "../src/signaling/hub.js";

async function waitMsg(ws: WebSocket): Promise<any> {
  return new Promise((res) => ws.once("message", (d) => res(JSON.parse(d.toString()))));
}

test("agent-online → connect → sdp relayed to agent", async () => {
  const db = openDb(":memory:");
  const users = new UsersRepo(db); const devices = new DevicesRepo(db);
  const u = users.create("a@b.com", "h"); const dev = devices.create(u.id, "PC");
  const registry = new Registry();
  const http = createServer(); const wss = new WebSocketServer({ server: http });
  attachSignaling(wss, { devices, config: { port: 0, jwtSecret: "s", relayPolicy: "relay-fallback", iceServers: [] }, registry });
  await new Promise<void>((r) => http.listen(0, r));
  const port = (http.address() as any).port;

  const agent = new WebSocket(`ws://localhost:${port}`);
  await new Promise((r) => agent.once("open", r));
  agent.send(JSON.stringify({ type: "agent-online", token: dev.token }));
  await new Promise((r) => setTimeout(r, 50));

  const web = new WebSocket(`ws://localhost:${port}`);
  await new Promise((r) => web.once("open", r));
  web.send(JSON.stringify({ type: "connect", deviceId: dev.id }));

  const incoming = await waitMsg(agent);
  expect(incoming.type).toBe("incoming");
  const sessionId = incoming.sessionId;

  // web 发 sdp offer，应被转发到 agent
  web.send(JSON.stringify({ type: "sdp", sessionId, sdp: { type: "offer", sdp: "v=0" } }));
  const relayed = await waitMsg(agent);
  expect(relayed).toMatchObject({ type: "sdp", sessionId });

  agent.close(); web.close(); wss.close(); http.close();
});
