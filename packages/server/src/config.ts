import type { RelayPolicy, IceServer } from "@rd/protocol";

export interface Config { port: number; jwtSecret: string; relayPolicy: RelayPolicy; iceServers: IceServer[]; }

const VALID_RELAY_POLICIES: RelayPolicy[] = ["direct-only", "relay-fallback", "force-relay"];

function parseRelayPolicy(value: string | undefined): RelayPolicy {
  if (!value) return "relay-fallback";
  if (!VALID_RELAY_POLICIES.includes(value as RelayPolicy)) {
    throw new Error(
      `Invalid RELAY_POLICY: "${value}". Expected one of: ${VALID_RELAY_POLICIES.join(", ")}`,
    );
  }
  return value as RelayPolicy;
}

function parseIceServers(value: string | undefined): IceServer[] {
  if (!value) return [{ urls: "stun:stun.l.google.com:19302" }];
  try {
    return JSON.parse(value);
  } catch {
    throw new Error(`Invalid ICE_SERVERS: must be valid JSON, got: "${value}"`);
  }
}

export function loadConfig(env: NodeJS.ProcessEnv = process.env): Config {
  return {
    port: Number(env.PORT ?? 8080),
    jwtSecret: env.JWT_SECRET ?? "dev-secret-change-me",
    relayPolicy: parseRelayPolicy(env.RELAY_POLICY),
    iceServers: parseIceServers(env.ICE_SERVERS),
  };
}
