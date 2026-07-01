import { expect, test, beforeEach } from "vitest";
import { openDb } from "../src/db.js";
import { UsersRepo } from "../src/repo/users.js";
import { DevicesRepo } from "../src/repo/devices.js";
import { buildApp } from "../src/app.js";

function makeApp() {
  const db = openDb(":memory:");
  return buildApp({
    users: new UsersRepo(db), devices: new DevicesRepo(db),
    config: { port: 0, jwtSecret: "test-secret", relayPolicy: "relay-fallback", iceServers: [] },
  });
}
let app: ReturnType<typeof makeApp>;
beforeEach(() => { app = makeApp(); });

async function token() {
  const res = await app.inject({ method: "POST", url: "/register", payload: { email: "a@b.com", password: "pw123456" } });
  return JSON.parse(res.body).token as string;
}

test("pair then list device", async () => {
  const jwt = await token();
  const pair = await app.inject({ method: "POST", url: "/devices/pair", headers: { authorization: `Bearer ${jwt}` }, payload: { name: "My PC" } });
  expect(pair.statusCode).toBe(200);
  const { deviceId, token: devToken } = JSON.parse(pair.body);
  expect(deviceId).toBeTruthy(); expect(devToken).toBeTruthy();

  const list = await app.inject({ method: "GET", url: "/devices", headers: { authorization: `Bearer ${jwt}` } });
  const { devices } = JSON.parse(list.body);
  expect(devices).toHaveLength(1);
  expect(devices[0]).toMatchObject({ id: deviceId, name: "My PC", online: false });
});

test("list without token → 401", async () => {
  const res = await app.inject({ method: "GET", url: "/devices" });
  expect(res.statusCode).toBe(401);
});
