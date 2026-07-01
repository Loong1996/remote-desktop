import type { FastifyInstance, FastifyRequest } from "fastify";
import type { DevicesRepo } from "../repo/devices.js";
import { verifyToken } from "../auth.js";

export function authUser(req: FastifyRequest, jwtSecret: string): string {
  const header = req.headers.authorization;
  if (!header?.startsWith("Bearer ")) throw new Error("no token");
  return verifyToken(header.slice(7), jwtSecret).userId;
}

/** isOnline: 由 Task 7 的在线表注入；本任务默认恒 false */
export function registerDeviceRoutes(
  app: FastifyInstance, devices: DevicesRepo, jwtSecret: string,
  isOnline: (deviceId: string) => boolean = () => false,
) {
  app.get("/devices", async (req, reply) => {
    let userId: string;
    try { userId = authUser(req, jwtSecret); } catch { return reply.code(401).send({ error: "unauthorized" }); }
    return { devices: devices.listByUser(userId).map(d => ({ id: d.id, name: d.name, online: isOnline(d.id) })) };
  });

  app.post("/devices/pair", async (req, reply) => {
    let userId: string;
    try { userId = authUser(req, jwtSecret); } catch { return reply.code(401).send({ error: "unauthorized" }); }
    const { name } = (req.body ?? {}) as { name?: string };
    if (!name) return reply.code(400).send({ error: "name required" });
    const d = devices.create(userId, name);
    return { deviceId: d.id, token: d.token };
  });
}
