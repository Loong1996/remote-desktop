import { WebSocketServer } from "ws";
import { openDb } from "./db.js";
import { UsersRepo } from "./repo/users.js";
import { DevicesRepo } from "./repo/devices.js";
import { loadConfig } from "./config.js";
import { buildApp } from "./app.js";
import { Registry } from "./signaling/registry.js";
import { attachSignaling } from "./signaling/hub.js";

if (process.env.NODE_ENV !== "test" && !process.env.JWT_SECRET) {
  console.error("JWT_SECRET environment variable is required (refusing to start with an insecure default).");
  process.exit(1);
}

const config = loadConfig();
const db = openDb(process.env.DB_PATH ?? "remote-desktop.db");
const users = new UsersRepo(db);
const devices = new DevicesRepo(db);
const registry = new Registry();

const app = buildApp({ users, devices, config, isOnline: (id) => registry.isOnline(id) });

// WS 复用 Fastify 内部的 http.Server（app.server）的 upgrade 事件，无需第二个 HTTP server
const wss = new WebSocketServer({ noServer: true });
attachSignaling(wss, { devices, config, registry });

Promise.resolve(app.ready()).then(() => {
  app.server.on("upgrade", (req, socket, head) => {
    wss.handleUpgrade(req, socket as any, head, (ws) => wss.emit("connection", ws, req));
  });
  app.listen({ port: config.port, host: "0.0.0.0" });
  console.log(`server on :${config.port}`);
}).catch((err: unknown) => {
  console.error(err);
  process.exit(1);
});
