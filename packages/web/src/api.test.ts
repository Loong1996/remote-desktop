import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { register, login, listDevices, pairDevice, API_BASE, deriveApiBase } from "./api.js";

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });
}

const fetchMock = vi.fn();

beforeEach(() => {
  fetchMock.mockReset();
  vi.stubGlobal("fetch", fetchMock);
});
afterEach(() => {
  vi.unstubAllGlobals();
});

describe("deriveApiBase", () => {
  const loc = { protocol: "https:", hostname: "example.test" };

  it("uses VITE_SERVER_URL verbatim when set (wins over page host)", () => {
    expect(deriveApiBase({ VITE_SERVER_URL: "http://198.51.100.7:5181" }, loc))
      .toBe("http://198.51.100.7:5181");
  });

  it("falls back to the page host + server port when VITE_SERVER_URL is unset", () => {
    // No baked-in IP: works from whatever address served the page (LAN, public, changed).
    expect(deriveApiBase({}, loc)).toBe("https://example.test:5181");
    expect(deriveApiBase({}, { protocol: "http:", hostname: "192.168.5.122" }))
      .toBe("http://192.168.5.122:5181");
  });

  it("honors VITE_SERVER_PORT for the page-host fallback", () => {
    expect(deriveApiBase({ VITE_SERVER_PORT: "9000" }, loc)).toBe("https://example.test:9000");
  });

  it("falls back to localhost outside a browser (no location)", () => {
    expect(deriveApiBase({}, undefined)).toBe("http://127.0.0.1:8080");
  });

  it("API_BASE resolves to the test-pinned VITE_SERVER_URL", () => {
    expect(API_BASE).toBe("http://127.0.0.1:8080");
  });
});

describe("login", () => {
  it("POSTs /login with email+password and returns the token", async () => {
    fetchMock.mockResolvedValueOnce(jsonResponse({ token: "jwt-123" }));

    const token = await login("a@b.com", "pw123456");

    expect(token).toBe("jwt-123");
    expect(fetchMock).toHaveBeenCalledTimes(1);
    const [url, init] = fetchMock.mock.calls[0];
    expect(url).toBe("http://127.0.0.1:8080/login");
    expect(init.method).toBe("POST");
    expect(init.headers["Content-Type"]).toBe("application/json");
    expect(JSON.parse(init.body)).toEqual({ email: "a@b.com", password: "pw123456" });
  });

  it("throws on non-ok responses", async () => {
    fetchMock.mockResolvedValueOnce(jsonResponse({ error: "bad credentials" }, 401));
    await expect(login("a@b.com", "nope")).rejects.toThrow();
  });
});

describe("register", () => {
  it("POSTs /register and returns the token", async () => {
    fetchMock.mockResolvedValueOnce(jsonResponse({ token: "jwt-reg" }));

    const token = await register("a@b.com", "pw123456");

    expect(token).toBe("jwt-reg");
    const [url, init] = fetchMock.mock.calls[0];
    expect(url).toBe("http://127.0.0.1:8080/register");
    expect(init.method).toBe("POST");
    expect(JSON.parse(init.body)).toEqual({ email: "a@b.com", password: "pw123456" });
  });
});

describe("listDevices", () => {
  it("GETs /devices with a Bearer token and returns the device array", async () => {
    fetchMock.mockResolvedValueOnce(
      jsonResponse({ devices: [{ id: "d1", name: "My PC", online: true }] }),
    );

    const devices = await listDevices("jwt-123");

    expect(devices).toEqual([{ id: "d1", name: "My PC", online: true }]);
    const [url, init] = fetchMock.mock.calls[0];
    expect(url).toBe("http://127.0.0.1:8080/devices");
    expect(init.headers.Authorization).toBe("Bearer jwt-123");
  });
});

describe("pairDevice", () => {
  it("POSTs /devices/pair with the name and Bearer token, returns id+token", async () => {
    fetchMock.mockResolvedValueOnce(jsonResponse({ deviceId: "d9", token: "dev-tok" }));

    const result = await pairDevice("jwt-123", "Laptop");

    expect(result).toEqual({ deviceId: "d9", token: "dev-tok" });
    const [url, init] = fetchMock.mock.calls[0];
    expect(url).toBe("http://127.0.0.1:8080/devices/pair");
    expect(init.method).toBe("POST");
    expect(init.headers.Authorization).toBe("Bearer jwt-123");
    expect(JSON.parse(init.body)).toEqual({ name: "Laptop" });
  });
});
