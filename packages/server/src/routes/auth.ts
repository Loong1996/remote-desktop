import type { FastifyInstance } from "fastify";
import type { UsersRepo } from "../repo/users.js";
import { hashPassword, verifyPassword, signToken } from "../auth.js";

interface Body { email?: string; password?: string; }

export function registerAuthRoutes(app: FastifyInstance, users: UsersRepo, jwtSecret: string) {
  app.post("/register", async (req, reply) => {
    const { email, password } = (req.body ?? {}) as Body;
    if (!email || !password || password.length < 6) return reply.code(400).send({ error: "invalid input" });
    if (users.findByEmail(email)) return reply.code(409).send({ error: "email exists" });
    const user = users.create(email, await hashPassword(password));
    return { token: signToken(user.id, jwtSecret) };
  });

  app.post("/login", async (req, reply) => {
    const { email, password } = (req.body ?? {}) as Body;
    if (!email || !password) return reply.code(400).send({ error: "invalid input" });
    const user = users.findByEmail(email);
    if (!user || !(await verifyPassword(password, user.passwordHash)))
      return reply.code(401).send({ error: "bad credentials" });
    return { token: signToken(user.id, jwtSecret) };
  });
}
