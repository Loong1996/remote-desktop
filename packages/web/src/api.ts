import type { Device } from "@rd/protocol";

/**
 * Resolve the API/signaling server base URL.
 *
 * Priority: explicit `VITE_SERVER_URL` build-time override → else, in a browser,
 * the same host that served the page on the server port (`VITE_SERVER_PORT`,
 * default 5181). Deriving from the page host means no specific IP/hostname is
 * baked in: the app works unchanged over LAN, a public IP, or a changed IP.
 * Falls back to localhost for non-browser (test/SSR) contexts.
 */
export function deriveApiBase(
  env: { VITE_SERVER_URL?: string; VITE_SERVER_PORT?: string },
  loc?: { protocol: string; hostname: string },
): string {
  if (env.VITE_SERVER_URL) return env.VITE_SERVER_URL;
  if (loc?.hostname) {
    return `${loc.protocol}//${loc.hostname}:${env.VITE_SERVER_PORT ?? "5181"}`;
  }
  return "http://127.0.0.1:8080";
}

/** Server base URL. See `deriveApiBase`. */
export const API_BASE: string = deriveApiBase(
  import.meta.env,
  typeof window !== "undefined" ? window.location : undefined,
);

export interface PairResult {
  deviceId: string;
  token: string;
}

async function request<T>(path: string, init: RequestInit): Promise<T> {
  const res = await fetch(`${API_BASE}${path}`, init);
  if (!res.ok) {
    let message = `request to ${path} failed (${res.status})`;
    try {
      const body = (await res.json()) as { error?: string };
      if (body?.error) message = body.error;
    } catch {
      // non-JSON error body; keep the default message
    }
    throw new Error(message);
  }
  return (await res.json()) as T;
}

function authHeaders(token: string): Record<string, string> {
  return { Authorization: `Bearer ${token}` };
}

/** Create an account and return the user JWT. */
export async function register(email: string, password: string): Promise<string> {
  const { token } = await request<{ token: string }>("/register", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ email, password }),
  });
  return token;
}

/** Authenticate and return the user JWT. */
export async function login(email: string, password: string): Promise<string> {
  const { token } = await request<{ token: string }>("/login", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ email, password }),
  });
  return token;
}

/** List the authenticated user's paired devices with online status. */
export async function listDevices(token: string): Promise<Device[]> {
  const { devices } = await request<{ devices: Device[] }>("/devices", {
    method: "GET",
    headers: authHeaders(token),
  });
  return devices;
}

/** Pair a new device; returns the device id and the device token for the agent. */
export async function pairDevice(token: string, name: string): Promise<PairResult> {
  return request<PairResult>("/devices/pair", {
    method: "POST",
    headers: { "Content-Type": "application/json", ...authHeaders(token) },
    body: JSON.stringify({ name }),
  });
}
