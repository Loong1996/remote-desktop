import type { Device } from "@rd/protocol";

/** Server base URL; override with VITE_SERVER_URL at build/dev time. */
export const API_BASE: string = import.meta.env.VITE_SERVER_URL ?? "http://127.0.0.1:8080";

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
