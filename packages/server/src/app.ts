import Fastify, { type FastifyInstance } from "fastify";
import type { UsersRepo } from "./repo/users.js";
import type { DevicesRepo } from "./repo/devices.js";
import type { Config } from "./config.js";
import { registerAuthRoutes } from "./routes/auth.js";
import { registerDeviceRoutes } from "./routes/devices.js";

export interface AppDeps { users: UsersRepo; devices: DevicesRepo; config: Config; isOnline?: (deviceId: string) => boolean; }

export function buildApp(deps: AppDeps): FastifyInstance {
  const app = Fastify({ logger: false });
  registerAuthRoutes(app, deps.users, deps.config.jwtSecret);
  registerDeviceRoutes(app, deps.devices, deps.config.jwtSecret, deps.isOnline);
  return app;
}
