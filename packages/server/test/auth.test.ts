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

test("register returns a token", async () => {
  const res = await app.inject({ method: "POST", url: "/register", payload: { email: "a@b.com", password: "pw123456" } });
  expect(res.statusCode).toBe(200);
  expect(JSON.parse(res.body).token).toBeTruthy();
});

test("login after register succeeds", async () => {
  await app.inject({ method: "POST", url: "/register", payload: { email: "a@b.com", password: "pw123456" } });
  const res = await app.inject({ method: "POST", url: "/login", payload: { email: "a@b.com", password: "pw123456" } });
  expect(res.statusCode).toBe(200);
});

test("login with wrong password fails 401", async () => {
  await app.inject({ method: "POST", url: "/register", payload: { email: "a@b.com", password: "pw123456" } });
  const res = await app.inject({ method: "POST", url: "/login", payload: { email: "a@b.com", password: "wrong" } });
  expect(res.statusCode).toBe(401);
});
