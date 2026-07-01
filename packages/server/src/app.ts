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
  // Allow the Vite dev server (and any localhost port) to call the API from the browser.
  // register() is queued and applied on ready(); app.inject() awaits ready(), so this
  // takes effect without buildApp needing to be async.
  app.register(cors, { origin: [/^http:\/\/localhost:\d+$/, /^http:\/\/127\.0\.0\.1:\d+$/] });
  registerAuthRoutes(app, deps.users, deps.config.jwtSecret);
  registerDeviceRoutes(app, deps.devices, deps.config.jwtSecret, deps.isOnline);
  return app;
}
