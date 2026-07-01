import { randomUUID } from "node:crypto";
import type { Db } from "../db.js";

export interface User { id: string; email: string; passwordHash: string; createdAt: number; }

export class UsersRepo {
  constructor(private db: Db) {}
  create(email: string, passwordHash: string): User {
    const user: User = { id: randomUUID(), email, passwordHash, createdAt: Date.now() };
    this.db.prepare("INSERT INTO users (id,email,passwordHash,createdAt) VALUES (?,?,?,?)")
      .run(user.id, user.email, user.passwordHash, user.createdAt);
    return user;
  }
  findByEmail(email: string): User | undefined {
    return this.db.prepare("SELECT * FROM users WHERE email = ?").get(email) as User | undefined;
  }
  findById(id: string): User | undefined {
    return this.db.prepare("SELECT * FROM users WHERE id = ?").get(id) as User | undefined;
  }
}
