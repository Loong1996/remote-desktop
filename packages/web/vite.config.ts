/// <reference types="vitest/config" />
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  server: { port: 5173 },
  test: {
    environment: "jsdom",
    globals: true,
    // Pin the API base for tests so URL assertions are stable regardless of the
    // jsdom host. deriveApiBase's host-relative fallback is unit-tested directly.
    env: { VITE_SERVER_URL: "http://127.0.0.1:8080" },
  },
});
