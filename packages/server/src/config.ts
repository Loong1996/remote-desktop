import type { RelayPolicy, IceServer } from "@rd/protocol";

export interface Config { port: number; jwtSecret: string; relayPolicy: RelayPolicy; iceServers: IceServer[]; }

export function loadConfig(env: NodeJS.ProcessEnv = process.env): Config {
  return {
    port: Number(env.PORT ?? 8080),
    jwtSecret: env.JWT_SECRET ?? "dev-secret-change-me",
    relayPolicy: (env.RELAY_POLICY as RelayPolicy) ?? "relay-fallback",
    iceServers: env.ICE_SERVERS ? JSON.parse(env.ICE_SERVERS) : [{ urls: "stun:stun.l.google.com:19302" }],
  };
}
