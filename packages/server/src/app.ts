import Fastify, { type FastifyInstance } from "fastify";
import cors from "@fastify/cors";
import type { UsersRepo } from "./repo/users.js";
import type { DevicesRepo } from "./repo/devices.js";
import type { Config } from "./config.js";
import { registerAuthRoutes } from "./routes/auth.js";
import { registerDeviceRoutes } from "./routes/devices.js";

export interface AppDeps { users: UsersRepo; devices: DevicesRepo; config: Config; isOnline?: (deviceId: string) => boolean; }

export function buildApp(deps: AppDeps): FastifyInstance {
  const app = Fastify({ logger: false });
  // CORS origins. By default allow the Vite dev server (any localhost/127.0.0.1
  // port) plus any explicit origins from CORS_ORIGINS (e.g. a LAN or public
  // address when serving the web app to other machines). A single "*" entry
  // disables the allowlist entirely (reflect any origin) — for serving to
  // browsers on changing/unknown addresses. register() is queued and applied on
  // ready(); app.inject() awaits ready(), so this needs no async buildApp.
  const extra = deps.config.corsOrigins ?? [];
  const origin = extra.includes("*")
    ? true
    : [/^http:\/\/localhost:\d+$/, /^http:\/\/127\.0\.0\.1:\d+$/, ...extra];
  app.register(cors, { origin });
  registerAuthRoutes(app, deps.users, deps.config.jwtSecret);
  registerDeviceRoutes(app, deps.devices, deps.config.jwtSecret, deps.isOnline);
  return app;
}
