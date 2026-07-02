import { expect, test, beforeEach } from "vitest";
import { openDb } from "../src/db.js";
import { UsersRepo } from "../src/repo/users.js";
import { DevicesRepo } from "../src/repo/devices.js";
import { buildApp } from "../src/app.js";

function makeApp(corsOrigins?: string[]) {
  const db = openDb(":memory:");
  return buildApp({
    users: new UsersRepo(db), devices: new DevicesRepo(db),
    config: { port: 0, jwtSecret: "test-secret", relayPolicy: "relay-fallback", iceServers: [], corsOrigins },
  });
}

let app: ReturnType<typeof makeApp>;
beforeEach(() => { app = makeApp(); });

test("responds with CORS header for the Vite dev origin", async () => {
  const res = await app.inject({
    method: "POST",
    url: "/login",
    headers: { origin: "http://localhost:5173" },
    payload: { email: "a@b.com", password: "pw123456" },
  });
  expect(res.headers["access-control-allow-origin"]).toBe("http://localhost:5173");
});

test("preflight OPTIONS from the Vite dev origin is allowed", async () => {
  const res = await app.inject({
    method: "OPTIONS",
    url: "/login",
    headers: {
      origin: "http://localhost:5173",
      "access-control-request-method": "POST",
    },
  });
  expect(res.statusCode).toBeLessThan(300);
  expect(res.headers["access-control-allow-origin"]).toBe("http://localhost:5173");
});

test("allows an explicit configured origin (e.g. a LAN address)", async () => {
  const lan = makeApp(["http://192.168.5.122:5180"]);
  const res = await lan.inject({
    method: "POST",
    url: "/login",
    headers: { origin: "http://192.168.5.122:5180" },
    payload: { email: "a@b.com", password: "pw123456" },
  });
  expect(res.headers["access-control-allow-origin"]).toBe("http://192.168.5.122:5180");
});

test("does not allow an origin that is neither localhost nor configured", async () => {
  const lan = makeApp(["http://192.168.5.122:5180"]);
  const res = await lan.inject({
    method: "POST",
    url: "/login",
    headers: { origin: "http://192.168.9.9:5180" },
    payload: { email: "a@b.com", password: "pw123456" },
  });
  expect(res.headers["access-control-allow-origin"]).toBeUndefined();
});
