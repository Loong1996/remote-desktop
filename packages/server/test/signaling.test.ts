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
import { signToken } from "../src/auth.js";

const JWT_SECRET = "s";

async function waitMsg(ws: WebSocket): Promise<any> {
  return new Promise((res) => ws.once("message", (d) => res(JSON.parse(d.toString()))));
}

/** Start a signaling server on an ephemeral port with an in-memory DB. */
async function startHub() {
  const db = openDb(":memory:");
  const users = new UsersRepo(db); const devices = new DevicesRepo(db);
  const registry = new Registry();
  const http = createServer(); const wss = new WebSocketServer({ server: http });
  attachSignaling(wss, { devices, config: { port: 0, jwtSecret: JWT_SECRET, relayPolicy: "relay-fallback", iceServers: [] }, registry });
  await new Promise<void>((r) => http.listen(0, r));
  const port = (http.address() as any).port;
  const teardown = () => { wss.close(); http.close(); };
  return { users, devices, port, teardown };
}

async function openWs(url: string): Promise<WebSocket> {
  const ws = new WebSocket(url);
  await new Promise((r) => ws.once("open", r));
  return ws;
}

test("agent-online → authenticated owner connect → sdp relayed to agent", async () => {
  const { users, devices, port, teardown } = await startHub();
  const u = users.create("a@b.com", "h"); const dev = devices.create(u.id, "PC");
  const jwt = signToken(u.id, JWT_SECRET);

  const agent = await openWs(`ws://localhost:${port}`);
  agent.send(JSON.stringify({ type: "agent-online", token: dev.token }));
  await new Promise((r) => setTimeout(r, 50));

  const web = await openWs(`ws://localhost:${port}?token=${jwt}`);
  web.send(JSON.stringify({ type: "connect", deviceId: dev.id }));

  const incoming = await waitMsg(agent);
  expect(incoming.type).toBe("incoming");
  const sessionId = incoming.sessionId;

  // web 发 sdp offer，应被转发到 agent
  web.send(JSON.stringify({ type: "sdp", sessionId, sdp: { type: "offer", sdp: "v=0" } }));
  const relayed = await waitMsg(agent);
  expect(relayed).toMatchObject({ type: "sdp", sessionId });

  agent.close(); web.close(); teardown();
});

test("authenticated user cannot connect to a device owned by someone else", async () => {
  const { users, devices, port, teardown } = await startHub();
  const owner = users.create("owner@b.com", "h");
  const dev = devices.create(owner.id, "OwnerPC");
  const attacker = users.create("attacker@b.com", "h");
  const attackerJwt = signToken(attacker.id, JWT_SECRET);

  // owner's agent is online
  const agent = await openWs(`ws://localhost:${port}`);
  agent.send(JSON.stringify({ type: "agent-online", token: dev.token }));
  await new Promise((r) => setTimeout(r, 50));

  // attacker (authenticated, but not the owner) tries to connect to owner's device
  const web = await openWs(`ws://localhost:${port}?token=${attackerJwt}`);
  web.send(JSON.stringify({ type: "connect", deviceId: dev.id }));

  const reply = await waitMsg(web);
  expect(reply).toMatchObject({ type: "error", code: "forbidden" });

  agent.close(); web.close(); teardown();
});

test("unauthenticated web (no token) cannot connect", async () => {
  const { users, devices, port, teardown } = await startHub();
  const u = users.create("a@b.com", "h"); const dev = devices.create(u.id, "PC");

  const agent = await openWs(`ws://localhost:${port}`);
  agent.send(JSON.stringify({ type: "agent-online", token: dev.token }));
  await new Promise((r) => setTimeout(r, 50));

  const web = await openWs(`ws://localhost:${port}`); // no ?token
  web.send(JSON.stringify({ type: "connect", deviceId: dev.id }));

  const reply = await waitMsg(web);
  expect(reply).toMatchObject({ type: "error", code: "unauthorized" });

  agent.close(); web.close(); teardown();
});

test("agent WS close notifies web with peer-left", async () => {
  const { users, devices, port, teardown } = await startHub();
  const u = users.create("a@b.com", "h");
  const dev = devices.create(u.id, "PC");
  const jwt = signToken(u.id, JWT_SECRET);

  const agent = await openWs(`ws://localhost:${port}`);
  agent.send(JSON.stringify({ type: "agent-online", token: dev.token }));
  await new Promise((r) => setTimeout(r, 50));

  const web = await openWs(`ws://localhost:${port}?token=${jwt}`);
  web.send(JSON.stringify({ type: "connect", deviceId: dev.id }));
  await waitMsg(web); // session-ready

  const left = waitMsg(web); // arm listener before closing
  agent.close();
  const msg = await left;
  expect(msg.type).toBe("peer-left");
  expect(typeof msg.sessionId).toBe("string");
  teardown();
});

test("web WS close notifies agent with peer-left", async () => {
  const { users, devices, port, teardown } = await startHub();
  const u = users.create("a@b.com", "h");
  const dev = devices.create(u.id, "PC");
  const jwt = signToken(u.id, JWT_SECRET);

  const agent = await openWs(`ws://localhost:${port}`);
  agent.send(JSON.stringify({ type: "agent-online", token: dev.token }));
  await new Promise((r) => setTimeout(r, 50));

  const web = await openWs(`ws://localhost:${port}?token=${jwt}`);
  web.send(JSON.stringify({ type: "connect", deviceId: dev.id }));
  await waitMsg(agent); // incoming
  await waitMsg(web); // session-ready

  const left = waitMsg(agent); // arm listener before closing
  web.close();
  const msg = await left;
  expect(msg.type).toBe("peer-left");
  teardown();
});
