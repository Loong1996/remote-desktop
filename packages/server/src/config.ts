import type { RelayPolicy, IceServer } from "@rd/protocol";

export interface Config { port: number; jwtSecret: string; relayPolicy: RelayPolicy; iceServers: IceServer[]; corsOrigins?: string[]; }

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

// Extra allowed browser origins beyond the built-in localhost/127.0.0.1 dev
// rules — e.g. a LAN address `http://192.168.1.20:5180` when serving the web
// app to other machines. Comma-separated exact origins; blanks ignored.
function parseCorsOrigins(value: string | undefined): string[] {
  if (!value) return [];
  return value.split(",").map((s) => s.trim()).filter((s) => s.length > 0);
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
    corsOrigins: parseCorsOrigins(env.CORS_ORIGINS),
  };
}
