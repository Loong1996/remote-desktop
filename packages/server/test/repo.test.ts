import { expect, test, beforeEach } from "vitest";
import { openDb } from "../src/db.js";
import { UsersRepo } from "../src/repo/users.js";
import { DevicesRepo } from "../src/repo/devices.js";

let users: UsersRepo, devices: DevicesRepo;
beforeEach(() => {
  const db = openDb(":memory:");
  users = new UsersRepo(db);
  devices = new DevicesRepo(db);
});

test("create and find user by email", () => {
  const u = users.create("a@b.com", "hash");
  expect(u.id).toBeTruthy();
  expect(users.findByEmail("a@b.com")?.id).toBe(u.id);
});

test("duplicate email throws", () => {
  users.create("a@b.com", "hash");
  expect(() => users.create("a@b.com", "hash2")).toThrow();
});

test("device gets id + token, findable by token", () => {
  const u = users.create("a@b.com", "hash");
  const d = devices.create(u.id, "My PC");
  expect(d.token.length).toBeGreaterThan(16);
  expect(devices.findByToken(d.token)?.id).toBe(d.id);
  expect(devices.listByUser(u.id).map(x => x.id)).toContain(d.id);
});
