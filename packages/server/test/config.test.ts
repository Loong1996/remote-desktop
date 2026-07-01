import { expect, test } from "vitest";
import { loadConfig } from "../src/config.js";

test("invalid RELAY_POLICY throws", () => {
  expect(() => loadConfig({ RELAY_POLICY: "bogus-policy" } as NodeJS.ProcessEnv)).toThrow(
    /RELAY_POLICY/,
  );
});

test("valid RELAY_POLICY (force-relay) is accepted", () => {
  const config = loadConfig({ RELAY_POLICY: "force-relay" } as NodeJS.ProcessEnv);
  expect(config.relayPolicy).toBe("force-relay");
});

test("malformed ICE_SERVERS throws", () => {
  expect(() => loadConfig({ ICE_SERVERS: "{not valid json" } as NodeJS.ProcessEnv)).toThrow(
    /ICE_SERVERS/,
  );
});

test("unset env yields defaults (port 8080, relay-fallback, default stun iceServers)", () => {
  const config = loadConfig({} as NodeJS.ProcessEnv);
  expect(config.port).toBe(8080);
  expect(config.relayPolicy).toBe("relay-fallback");
  expect(config.iceServers).toEqual([{ urls: "stun:stun.l.google.com:19302" }]);
});
